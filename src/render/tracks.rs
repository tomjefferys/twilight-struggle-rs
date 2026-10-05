//! The `t` modal: every track the game keeps — Space Race, Military Operations, DEFCON, VP and
//! the Turn/Action Round — as tabs under one strip. Pure `Canvas` producers; each tab is a
//! [`modal_box`], so its text wraps rather than overflowing.

use crate::country::Superpower;
use crate::ops::defcon_banned;
use crate::status::{hand_size_for_turn, rounds_for_turn, GameStatus};

use super::space::{render_space_track_sized, side_color, TRACK_WIDTH};
use super::{modal_box, modal_text_width, wrap_styled, Canvas, Color, Style, MODAL_PADDING};

/// One tab of the `t` tracks modal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackTab {
    Space,
    Military,
    Defcon,
    Vp,
    Turn,
}

impl TrackTab {
    pub const ALL: [TrackTab; 5] = [TrackTab::Space, TrackTab::Military, TrackTab::Defcon, TrackTab::Vp, TrackTab::Turn];

    pub fn label(self) -> &'static str {
        match self {
            TrackTab::Space => "Space Race",
            TrackTab::Military => "Military Ops",
            TrackTab::Defcon => "DEFCON",
            TrackTab::Vp => "VP",
            TrackTab::Turn => "Turn",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&t| t == self).expect("every tab is in ALL")
    }

    /// The next tab, wrapping.
    pub fn next(self) -> TrackTab {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    /// The previous tab, wrapping.
    pub fn prev(self) -> TrackTab {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

fn plain() -> Style {
    Style::default()
}

fn muted() -> Style {
    Style::color(Color::Muted)
}

fn blank() -> (String, Style) {
    (String::new(), plain())
}

/// A track tab's box: `lines` plus, when `hint` isn't empty, a right-aligned key hint at the
/// bottom. Blank rows pad it out to `min_height` (the whole box, border included), so every
/// tab of the modal is the same size.
fn track_box(title: &str, lines: Vec<(String, Style)>, hint: &str, min_height: usize) -> Canvas {
    let width = modal_text_width(TRACK_WIDTH);
    let mut lines = wrap_styled(lines, width);
    let footer = if hint.is_empty() { 0 } else { 2 };
    while lines.len() + footer + 2 < min_height {
        lines.push(blank());
    }
    if !hint.is_empty() {
        lines.push(blank());
        lines.push((format!("{hint:>width$}"), muted()));
    }
    modal_box(title, "", TRACK_WIDTH, plain(), false, lines)
}

/// The Military Operations track: each side's marker (0-5), DEFCON, and what the end of the
/// turn does with them — the shortfall against DEFCON is paid to the opponent in VP.
fn military_box(status: &GameStatus, hint: &str, min_height: usize) -> Canvas {
    let mut lines: Vec<(String, Style)> = Vec::new();
    lines.push((format!("DEFCON {} — each side needs that many Military Ops by the end of the turn", status.defcon), plain()));
    lines.push(blank());
    for side in [Superpower::Us, Superpower::Ussr] {
        let ops = status.military_ops(side);
        let bar: String = (0..=5).map(|n| if n == ops { "◆" } else if n < ops { "■" } else { "□" }).collect::<Vec<_>>().join(" ");
        let name = if side == Superpower::Us { "USA " } else { "USSR" };
        lines.push((format!("{name}  {bar}   {ops} of 5"), Style::color(side_color(side)).bold()));
        let short = status.military_shortfall(side);
        let consequence = if short == 0 {
            format!("      meets DEFCON {} — no penalty", status.defcon)
        } else {
            format!("      {short} short of DEFCON {} — {} gets {short} VP at the end of the turn", status.defcon, side.opponent())
        };
        lines.push((consequence, if short == 0 { muted() } else { Style::color(side_color(side.opponent())) }));
        lines.push(blank());
    }
    let swing = status.military_shortfall(Superpower::Ussr) - status.military_shortfall(Superpower::Us);
    let (net, color) = match swing {
        0 => ("Net at the end of the turn: no VP change".to_string(), Color::Muted),
        n if n > 0 => (format!("Net at the end of the turn: USA +{n} VP"), Color::Us),
        n => (format!("Net at the end of the turn: USSR +{} VP", -n), Color::Ussr),
    };
    lines.push((net, Style::color(color).bold()));
    lines.push(("Both tracks reset to 0 when the turn ends. Coups add their ops (max 5); a war event adds its Military Ops too.".to_string(), muted()));
    track_box("Military Operations", lines, hint, min_height)
}

/// The DEFCON track, 5 down to 1: where it stands and which regions are closed to coups and
/// realignments at each level (rule 6.1.3).
fn defcon_box(status: &GameStatus, hint: &str, min_height: usize) -> Canvas {
    let mut lines: Vec<(String, Style)> = Vec::new();
    for level in (1..=5u8).rev() {
        let here = status.defcon == level;
        let marker = if here { "◆" } else { " " };
        let what = match level {
            5 => "Peace — no region is closed".to_string(),
            1 => "Nuclear war — the phasing player loses the game".to_string(),
            _ => {
                let newly: Vec<String> = defcon_banned(level).into_iter().filter(|r| !defcon_banned(level + 1).contains(r)).map(|r| r.to_string()).collect();
                format!("{} closed to coups and realignments", newly.join(", "))
            }
        };
        let style = if here { Style::color(Color::Selected).bold() } else if level > status.defcon { muted() } else { plain() };
        lines.push((format!("{marker} {level}  {what}"), style));
    }
    lines.push(blank());
    let closed = defcon_banned(status.defcon);
    let now = if closed.is_empty() { "nothing is closed".to_string() } else { closed.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(", ") + " closed" };
    lines.push((format!("DEFCON {} — {now}", status.defcon), Style::default().bold()));
    lines.push((format!("Each side needs {} Military Ops by the end of the turn.", status.defcon), plain()));
    lines.push(blank());
    lines.push(("Influence placement is never restricted. A coup in a battleground lowers DEFCON by 1; it rises by 1 at the end of every turn (max 5).".to_string(), muted()));
    track_box("DEFCON", lines, hint, min_height)
}

/// The VP track, −20 to +20 (positive favours the USA), with the marker and who leads.
fn vp_box(status: &GameStatus, hint: &str, min_height: usize) -> Canvas {
    const CELLS: usize = 41;
    let at = (status.vp.clamp(-20, 20) + 20) as usize;
    let leader = match status.vp.signum() {
        1 => Some(Superpower::Us),
        -1 => Some(Superpower::Ussr),
        _ => None,
    };
    let bar: String = (0..CELLS).map(|i| if i == 20 { '┼' } else { '─' }).collect();
    let mut scale = vec![' '; CELLS];
    for (i, c) in "20".chars().enumerate() {
        scale[i] = c;
        scale[CELLS - 2 + i] = c;
    }
    scale[20] = '0';
    let mut marker = vec![' '; CELLS];
    marker[at] = '◆';
    let leader_style = leader.map_or(plain().bold(), |s| Style::color(side_color(s)).bold());

    let mut lines: Vec<(String, Style)> = Vec::new();
    let (left, right) = ("◀ USSR wins at −20", "USA wins at +20 ▶");
    let gap = CELLS - left.chars().count() - right.chars().count();
    lines.push((format!("{left}{}{right}", " ".repeat(gap)), muted()));
    lines.push((scale.iter().collect(), muted()));
    lines.push((bar, plain()));
    lines.push((marker.iter().collect(), leader_style));
    lines.push(blank());
    let summary = match leader {
        Some(side) => format!("{} VP to {side} — {side} leads", status.vp.abs()),
        None => "0 VP — level".to_string(),
    };
    lines.push((summary, leader_style));
    lines.push(blank());
    lines.push(("Reaching ±20 ends the game at once. After turn 10, final scoring is added and the side ahead wins; a tie is a draw.".to_string(), muted()));
    track_box("Victory Points", lines, hint, min_height)
}

/// The turn and action round: where the game is among the ten turns, with each turn's era,
/// action rounds and hand size.
fn turn_box(status: &GameStatus, hint: &str, min_height: usize) -> Canvas {
    let era = |t: u8| match t {
        1..=3 => "Early War",
        4..=7 => "Mid War",
        _ => "Late War",
    };
    let mut lines: Vec<(String, Style)> = Vec::new();
    lines.push((format!("Turn {} of 10 — {}", status.turn, era(status.turn)), Style::default().bold()));
    let round = if status.in_headline() {
        "Headline phase — both sides choose a headline card".to_string()
    } else {
        let n = status.action_rounds_per_turn;
        let extra = if status.action_round > n { " (an extra round)" } else { "" };
        format!("Action round {} of {n}{extra} — {} to act", status.action_round, status.active)
    };
    lines.push((round, Style::color(side_color(status.active))));
    lines.push(blank());
    for t in 1..=10u8 {
        let marker = if t == status.turn { "◆" } else { " " };
        let style = if t == status.turn { Style::color(Color::Selected).bold() } else if t < status.turn { muted() } else { plain() };
        lines.push((format!("{marker} {t:>2}  {:<10} {} action rounds   hand {}", era(t), rounds_for_turn(t), hand_size_for_turn(t)), style));
    }
    lines.push(blank());
    lines.push(("End of each turn: Military Ops penalty, then a scoring card still held loses the game, the China Card flips face up, DEFCON rises by 1 and both hands are dealt back up. After turn 10, final scoring.".to_string(), muted()));
    track_box("Turn", lines, hint, min_height)
}

pub fn render_military_track(status: &GameStatus, hint: &str) -> Canvas {
    military_box(status, hint, 0)
}

pub fn render_defcon_track(status: &GameStatus, hint: &str) -> Canvas {
    defcon_box(status, hint, 0)
}

pub fn render_vp_track(status: &GameStatus, hint: &str) -> Canvas {
    vp_box(status, hint, 0)
}

pub fn render_turn_track(status: &GameStatus, hint: &str) -> Canvas {
    turn_box(status, hint, 0)
}

fn tab_box(status: &GameStatus, tab: TrackTab, hint: &str, min_height: usize) -> Canvas {
    match tab {
        TrackTab::Space => render_space_track_sized(status, hint, min_height),
        TrackTab::Military => military_box(status, hint, min_height),
        TrackTab::Defcon => defcon_box(status, hint, min_height),
        TrackTab::Vp => vp_box(status, hint, min_height),
        TrackTab::Turn => turn_box(status, hint, min_height),
    }
}

/// The `t` modal: a tab strip above the chosen track. Every tab is as tall as the tallest, so
/// switching tabs never resizes the modal.
pub fn render_tracks(status: &GameStatus, tab: TrackTab, hint: &str) -> Canvas {
    let height = TrackTab::ALL.iter().map(|&t| tab_box(status, t, hint, 0).height()).max().unwrap_or(0);
    let body = tab_box(status, tab, hint, height);
    let strip: Vec<String> = TrackTab::ALL
        .iter()
        .map(|&t| if t == tab { format!("[{}]", t.label()) } else { format!(" {} ", t.label()) })
        .collect();
    let mut canvas = Canvas::new(TRACK_WIDTH, body.height() + 1);
    canvas.put(0, 1 + MODAL_PADDING, &strip.join(" "), plain().bold());
    canvas.blit(&body, 1, 0);
    canvas
}
