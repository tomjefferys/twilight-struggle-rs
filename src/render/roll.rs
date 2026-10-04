//! The post-roll result modal for a resolved realignment roll or coup
//! attempt — interactive mode's answer to "what did that just do?". A
//! realignment or coup roll is irreversible the instant it happens (see
//! `game.rs`'s own doc), so it deserves more than the dense one-line
//! summary [`super::roll_result_line`]/[`super::coup_result_line`] give it
//! (those stay as they are — the REPL still prints them inline). This view
//! spells the same event out as a titled box: the dice and modifiers that
//! went into each side's total, who won and by how much, and the resulting
//! influence (and control, if it flipped) in the target country. Meant to
//! be blitted centred over whichever screen [`crate::interactive`] is
//! showing and dismissed with Enter — nothing here touches the terminal.

use crate::country::{CountryId, Superpower};
use crate::game::RollOutcome;
use crate::log::CoupAftermath;
use crate::map::WorldMap;
use crate::ops::Modifiers;

use super::{put_border_title, wrap, Canvas, Color, Style};

/// Fixed content width — like [`super::card::render_card`]'s own
/// `CARD_WIDTH`, nothing about this box's width should depend on the
/// country or dice involved. A modifier breakdown can run long (several
/// reasons at once), so unlike a dice total this isn't a hard cap on the
/// box's own first few lines — see [`push_roll_lines`], which wraps it
/// onto its own line(s) instead of widening the box to fit it.
const ROLL_WIDTH: usize = 56;
/// Columns of padding inside the border on each side.
const PADDING: usize = 2;

/// Everything [`render_roll_result`] needs to describe one resolved roll:
/// who acted, what [`crate::game::Game::roll`] returned, and each side's
/// influence in the target country *before* the roll — the result itself
/// only carries what changed, not the starting point it changed from.
#[derive(Debug, Clone, PartialEq)]
pub struct RollReport {
    pub side: Superpower,
    pub outcome: RollOutcome,
    /// `(US influence, USSR influence)` in the target, as they stood the
    /// instant before this roll.
    pub before: (u8, u8),
    /// What a coup attempt set off beyond its own result — a DEFCON drop,
    /// Nuclear Subs sparing it, Yuri and Samantha's VP. Always `None` for
    /// a realignment.
    pub aftermath: Option<CoupAftermath>,
}

impl RollReport {
    /// The country this roll targeted — [`RollOutcome::Realign`]'s and
    /// [`RollOutcome::Coup`]'s own `target` field, whichever it is.
    pub fn target(&self) -> CountryId {
        match &self.outcome {
            RollOutcome::Realign(r) => r.target,
            RollOutcome::Coup(c) => c.target,
            RollOutcome::War(w) => w.target,
        }
    }
}

/// Draws `report` as a titled box. `queue_pos`, when more reports are
/// queued behind this one — an AI's turn can roll several times in a row
/// before handing back control — is `Some((n, total))` and adds a
/// `n of total` counter to the dismiss hint, so stepping through them with
/// Enter says how many are left.
pub fn render_roll_result(map: &WorldMap, report: &RollReport, queue_pos: Option<(usize, usize)>) -> Canvas {
    let country = &map.country(report.target()).name;
    let stability = map.country(report.target()).stability;
    let (before_us, before_ussr) = report.before;

    let text_width = ROLL_WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();
    let title;
    let after_us;
    let after_ussr;

    let border_color = match &report.outcome {
        RollOutcome::Realign(result) => {
            title = format!("Realignment · {country}");
            let acting = report.side;
            let opposing = acting.opponent();
            let acting_total = result.acting_die as i32 + result.acting_mods.total() as i32;
            let opposing_total = result.opposing_die as i32 + result.opposing_mods.total() as i32;
            push_roll_lines(&mut lines, acting, result.acting_die, &result.acting_mods, acting_total, text_width);
            push_roll_lines(&mut lines, opposing, result.opposing_die, &result.opposing_mods, opposing_total, text_width);
            lines.push((String::new(), Style::default()));

            let (outcome_line, outcome_style) = match result.loser {
                None => (format!("TIE — no influence removed in {country}"), Style::color(Color::Muted).bold()),
                Some(loser) if loser == opposing => {
                    (format!("{acting} WINS — removes {} {opposing} influence", result.removed), Style::color(side_color(acting)).bold())
                }
                Some(_) => (
                    format!("{opposing} WINS the roll — {acting} loses {} of its own influence", result.removed),
                    Style::color(side_color(opposing)).bold(),
                ),
            };
            push_text(&mut lines, &outcome_line, outcome_style, text_width);

            after_us = before_us.saturating_sub(if result.loser == Some(Superpower::Us) { result.removed } else { 0 });
            after_ussr = before_ussr.saturating_sub(if result.loser == Some(Superpower::Ussr) { result.removed } else { 0 });

            if let Some(loser) = result.loser {
                lines.push((String::new(), Style::default()));
                let (before, after) = if loser == Superpower::Us { (before_us, after_us) } else { (before_ussr, after_ussr) };
                push_text(&mut lines, &influence_change_line(loser, country, before, after), Style::default(), text_width);
            }

            match result.loser {
                None => Color::Muted,
                Some(loser) if loser == opposing => side_color(acting),
                Some(_) => side_color(opposing),
            }
        }
        RollOutcome::War(_) => unreachable!("a war result is shown by render_war_result, never as a RollReport"),
        RollOutcome::Coup(result) => {
            title = format!("Coup · {country}");
            let acting = report.side;
            let opposing = acting.opponent();
            let modified = result.die as i32 + result.ops as i32 + result.modifier as i32;
            let modifier = if result.modifier == 0 { String::new() } else { format!(" {:+}", result.modifier) };
            lines.push((format!("{acting}  rolled {} + {} ops{modifier} = {modified}", result.die, result.ops), Style::default()));
            lines.push((format!("  vs target {} (stability {stability} ×2)", result.target_number), Style::color(Color::Muted)));
            lines.push((String::new(), Style::default()));

            let success = result.success();
            let (outcome_line, outcome_style) = if success {
                (format!("{acting} COUP SUCCEEDS by {}", result.margin), Style::color(side_color(acting)).bold())
            } else {
                (format!("COUP FAILS — {modified} is not greater than {}", result.target_number), Style::color(Color::Muted).bold())
            };
            push_text(&mut lines, &outcome_line, outcome_style, text_width);

            after_us = if acting == Superpower::Us { before_us + result.added } else { before_us.saturating_sub(result.removed) };
            after_ussr = if acting == Superpower::Ussr { before_ussr + result.added } else { before_ussr.saturating_sub(result.removed) };

            if success {
                lines.push((String::new(), Style::default()));
                if result.removed > 0 {
                    let (before, after) = if opposing == Superpower::Us { (before_us, after_us) } else { (before_ussr, after_ussr) };
                    push_text(&mut lines, &influence_change_line(opposing, country, before, after), Style::default(), text_width);
                }
                if result.added > 0 {
                    let (before, after) = if acting == Superpower::Us { (before_us, after_us) } else { (before_ussr, after_ussr) };
                    push_text(&mut lines, &influence_change_line(acting, country, before, after), Style::default(), text_width);
                }
            }

            if success {
                side_color(acting)
            } else {
                Color::Muted
            }
        }
    };

    let before_control = controller_from(before_us, before_ussr, stability);
    let after_control = controller_from(after_us, after_ussr, stability);
    if before_control != after_control {
        let text = format!("Control: {} → {}", control_label(before_control), control_label(after_control));
        push_text(&mut lines, &text, Style::default().bold(), text_width);
    }

    if let Some(aftermath) = report.aftermath.filter(|a| !a.is_empty()) {
        lines.push((String::new(), Style::default()));
        if let Some((before, after)) = aftermath.mil_ops {
            push_text(&mut lines, &format!("Military Operations {before} → {after}"), Style::color(Color::Muted).bold(), text_width);
        }
        if let Some((before, after)) = aftermath.defcon {
            push_text(&mut lines, &format!("DEFCON {before} → {after} (battleground coup)"), Style::color(Color::Muted).bold(), text_width);
        }
        if aftermath.defcon_spared {
            push_text(&mut lines, "Nuclear Subs: DEFCON unchanged", Style::color(Color::Muted).bold(), text_width);
        }
        if let Some((delta, vp_after)) = aftermath.vp {
            let side = if delta > 0 { Superpower::Us } else { Superpower::Ussr };
            let text = format!("Yuri and Samantha: {:+} VP to the {side} (now {vp_after})", delta.abs());
            push_text(&mut lines, &text, Style::color(side_color(side)).bold(), text_width);
        }
    }

    lines.push((String::new(), Style::default()));
    let hint = match queue_pos {
        Some((n, total)) => format!("Enter to continue · {n} of {total}"),
        None => "Enter to continue".to_string(),
    };
    lines.push((format!("{hint:>text_width$}"), Style::color(Color::Muted)));

    let box_height = 2 + lines.len();
    let mut canvas = Canvas::new(ROLL_WIDTH, box_height);
    canvas.draw_thick_box(0, 0, ROLL_WIDTH, box_height, Style::color(border_color));
    put_border_title(&mut canvas, 0, 0, &title, Style::default().bold(), "", Style::default(), ROLL_WIDTH);

    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }

    canvas
}

/// Appends one side's roll to `lines`: a short `{side} rolled {die} {mod}
/// = {total}` line, followed by its modifier breakdown
/// (`Modifiers::reasons`, the same list `super::modifier_line` draws
/// from) word-wrapped onto its own indented line(s) — kept separate so
/// the numeric line's own width never depends on how many reasons happen
/// to apply.
fn push_roll_lines(lines: &mut Vec<(String, Style)>, side: Superpower, die: u8, mods: &Modifiers, total: i32, text_width: usize) {
    lines.push((format!("{side}  rolled {die}  {:+}  = {total}", mods.total()), Style::default()));
    let reasons = mods.reasons();
    let detail = if reasons.is_empty() { "no modifiers".to_string() } else { reasons.join(" · ") };
    for line in wrap(&detail, text_width.saturating_sub(2)) {
        lines.push((format!("  {line}"), Style::color(Color::Muted)));
    }
}

/// Pushes `text`, word-wrapped to `text_width`, onto `lines` — every
/// wrapped line keeps the same `style`, so a bold/coloured outcome line
/// stays that way across however many lines it wraps to. Used for every
/// free-text line below the roll breakdown itself (the outcome sentence,
/// an influence change, the control line): none of them has a fixed
/// bound on length, since a country's own name — or, for the outcome
/// sentence, the removed/added count — can push an otherwise-short line
/// past the box's own width.
fn push_text(lines: &mut Vec<(String, Style)>, text: &str, style: Style, text_width: usize) {
    for line in wrap(text, text_width) {
        lines.push((line, style));
    }
}

fn influence_change_line(side: Superpower, country: &str, before: u8, after: u8) -> String {
    format!("{side} influence in {country}: {before} → {after}")
}

fn side_color(side: Superpower) -> Color {
    match side {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    }
}

/// Mirrors [`crate::board::Board::controller`]'s own rule, over bare
/// influence numbers rather than a live [`crate::board::Board`] — this
/// view needs it for a hypothetical "before" state the real board never
/// held.
fn controller_from(us: u8, ussr: u8, stability: u8) -> Option<Superpower> {
    let stability = stability as u16;
    let us = us as u16;
    let ussr = ussr as u16;
    if us >= ussr + stability {
        Some(Superpower::Us)
    } else if ussr >= us + stability {
        Some(Superpower::Ussr)
    } else {
        None
    }
}

fn control_label(controller: Option<Superpower>) -> &'static str {
    match controller {
        Some(Superpower::Us) => "USA",
        Some(Superpower::Ussr) => "USSR",
        None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::Superpower::*;
    use crate::map::{Found, WorldMap};
    use crate::log::CoupAftermath;
    use crate::ops::{CoupResult, RollResult};
    use crate::render::ColorMode;

    fn map() -> WorldMap {
        WorldMap::standard().unwrap()
    }

    fn poland(map: &WorldMap) -> CountryId {
        match map.find("Poland") {
            Found::One(id) => id,
            _ => panic!("Poland should resolve uniquely"),
        }
    }

    fn zero_mods() -> Modifiers {
        Modifiers { adjacent_controlled: 0, more_influence: false, superpower_adjacent: false, iran_contra: false }
    }

    #[test]
    fn a_realignment_win_names_the_winner_and_the_new_influence() {
        let map = map();
        let id = poland(&map);
        let result = RollResult {
            target: id,
            acting_die: 4,
            acting_mods: Modifiers { adjacent_controlled: 0, more_influence: true, superpower_adjacent: false, iran_contra: false },
            opposing_die: 2,
            opposing_mods: zero_mods(),
            loser: Some(Us),
            removed: 3,
        };
        let report = RollReport { side: Ussr, outcome: RollOutcome::Realign(result), before: (3, 1), aftermath: None };
        let canvas = render_roll_result(&map, &report, None);
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("USSR WINS"), "{text}");
        assert!(text.contains("Poland: 3 → 0"), "{text}");
        for line in text.lines() {
            assert!(line.chars().count() <= ROLL_WIDTH, "line too wide: {line:?}");
        }
    }

    #[test]
    fn a_long_outcome_sentence_wraps_instead_of_overrunning_the_box() {
        // Regression: "USSR WINS the roll — USA loses 0 of its own
        // influence" (acting side USA out-rolled, removed capped at 0
        // since USA had none there to lose) is 53 chars, past the box's
        // own 50-char content width — it used to be pushed as a single
        // unwrapped line and spill past the right border.
        let map = map();
        let id = poland(&map);
        let result = RollResult {
            target: id,
            acting_die: 1,
            acting_mods: zero_mods(),
            opposing_die: 6,
            opposing_mods: zero_mods(),
            loser: Some(Us),
            removed: 0,
        };
        let report = RollReport { side: Us, outcome: RollOutcome::Realign(result), before: (0, 2), aftermath: None };
        let canvas = render_roll_result(&map, &report, None);
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("USSR WINS the roll"), "{text}");
        for line in text.lines() {
            assert!(line.chars().count() <= ROLL_WIDTH, "line too wide: {line:?}");
        }
    }

    #[test]
    fn a_realignment_tie_removes_nothing() {
        let map = map();
        let id = poland(&map);
        let result = RollResult {
            target: id,
            acting_die: 3,
            acting_mods: zero_mods(),
            opposing_die: 3,
            opposing_mods: zero_mods(),
            loser: None,
            removed: 0,
        };
        let report = RollReport { side: Ussr, outcome: RollOutcome::Realign(result), before: (2, 2), aftermath: None };
        let canvas = render_roll_result(&map, &report, None);
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("TIE"), "{text}");
        assert!(!text.contains("Control:"), "{text}");
    }

    #[test]
    fn a_successful_coup_shows_both_sides_changing_and_any_control_flip() {
        let map = map();
        let id = poland(&map);
        // Poland's stability is 3: USSR controls it at (0, 3) (3 >= 0+3),
        // and a coup that fully clears USSR's influence while adding 3 of
        // its own (3 >= 0+3) flips control the other way.
        let result = CoupResult { target: id, die: 6, ops: 5, modifier: 0, target_number: 5, margin: 6, removed: 3, added: 3 };
        let report = RollReport { side: Us, outcome: RollOutcome::Coup(result), before: (0, 3), aftermath: None };
        let canvas = render_roll_result(&map, &report, Some((1, 2)));
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("COUP SUCCEEDS"), "{text}");
        assert!(text.contains("USSR influence in Poland: 3 → 0"), "{text}");
        assert!(text.contains("USA influence in Poland: 0 → 3"), "{text}");
        assert!(text.contains("Control: USSR → USA"), "{text}");
        assert!(text.contains("1 of 2"), "{text}");
        for line in text.lines() {
            assert!(line.chars().count() <= ROLL_WIDTH, "line too wide: {line:?}");
        }
    }

    #[test]
    fn a_failed_coup_names_the_shortfall_and_changes_nothing() {
        let map = map();
        let id = poland(&map);
        let result = CoupResult { target: id, die: 1, ops: 2, modifier: 0, target_number: 6, margin: 0, removed: 0, added: 0 };
        let report = RollReport { side: Us, outcome: RollOutcome::Coup(result), before: (0, 2), aftermath: None };
        let canvas = render_roll_result(&map, &report, None);
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("COUP FAILS"), "{text}");
        assert!(!text.contains("Control:"), "{text}");
        assert!(!text.contains("influence in Poland"), "{text}");
    }

    #[test]
    fn a_coup_aftermath_lists_the_defcon_drop_and_yuris_vp() {
        let map = map();
        let id = poland(&map);
        let result = CoupResult { target: id, die: 6, ops: 2, modifier: 0, target_number: 5, margin: 3, removed: 3, added: 0 };
        let aftermath = CoupAftermath { defcon: Some((4, 3)), defcon_spared: false, vp: Some((-1, -2)), mil_ops: None };
        let report = RollReport { side: Us, outcome: RollOutcome::Coup(result), before: (0, 3), aftermath: Some(aftermath) };
        let text = render_roll_result(&map, &report, None).render(ColorMode::Never);
        assert!(text.contains("DEFCON 4 → 3"), "{text}");
        assert!(text.contains("Yuri and Samantha: +1 VP to the USSR (now -2)"), "{text}");

        let spared = CoupAftermath { defcon_spared: true, ..Default::default() };
        let report = RollReport { aftermath: Some(spared), ..report };
        let text = render_roll_result(&map, &report, None).render(ColorMode::Never);
        assert!(text.contains("Nuclear Subs: DEFCON unchanged"), "{text}");
    }

    #[test]
    fn a_coup_roll_modifier_is_part_of_the_modified_roll_shown() {
        let map = map();
        let id = poland(&map);
        let result = CoupResult { target: id, die: 3, ops: 2, modifier: 1, target_number: 5, margin: 1, removed: 1, added: 0 };
        let report = RollReport { side: Ussr, outcome: RollOutcome::Coup(result), before: (2, 0), aftermath: None };
        let text = render_roll_result(&map, &report, None).render(ColorMode::Never);
        assert!(text.contains("rolled 3 + 2 ops +1 = 6"), "{text}");
    }
}
