//! The "pass the keyboard" screen shown whenever the player who has to act changes (hotseat
//! play): it names the side up next, says where the game stands — turn, era, action round —
//! and lists what happened since that side last acted. It shows only public information, and
//! while it is up the hand strip is drawn face-down ([`render_hand_hidden`]).

use crate::cards::CardCatalog;
use crate::country::Superpower;
use crate::game::{Game, Phase, Trap};
use crate::status::GameStatus;

use super::{lasting_effect_line, modal_box, ongoing_effect_line, vp_line, Canvas, Color, Style};

const HANDOVER_WIDTH: usize = 64;

fn side_color(side: Superpower) -> Color {
    match side {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    }
}

fn era(turn: u8) -> &'static str {
    match turn {
        0..=3 => "Early War",
        4..=7 => "Mid War",
        _ => "Late War",
    }
}

/// Where the game is, in one line: `Turn 4 of 10 · Mid War · Action round 3 of 7`.
pub fn round_line(game: &Game) -> String {
    let status = game.status();
    let turn = format!("Turn {} of 10 · {}", status.turn, era(status.turn));
    match game.phase() {
        Phase::Setup => format!("{turn} · Setup: opening placement"),
        Phase::Headline => format!("{turn} · Headline phase"),
        Phase::TurnEnd => format!("{turn} · End of turn"),
        Phase::ActionRounds if game.awaiting_discard().is_some() => format!("{turn} · End of turn: Eagle/Bear has Landed"),
        Phase::ActionRounds => {
            let rounds = status.rounds_for(game.active());
            if status.action_round > status.action_rounds_per_turn {
                format!("{turn} · Extra action round {} of {rounds}", status.action_round)
            } else {
                format!("{turn} · Action round {} of {rounds}", status.action_round)
            }
        }
    }
}

/// What the incoming `side` is being asked to do.
fn task_line(game: &Game, cards: &CardCatalog, side: Superpower) -> String {
    if game.awaiting_discard() == Some(side) {
        return "Eagle/Bear has Landed: you may discard a card from your hand.".to_string();
    }
    if game.decider() != game.active() {
        return match game.card_in_play() {
            Some(card) => format!("{side} must respond to {}'s event.", cards.card(card).name),
            None => format!("{side} has a decision to make."),
        };
    }
    match game.phase() {
        Phase::Setup => format!("{side} places the opening influence."),
        Phase::Headline => format!("{side} chooses a headline card."),
        _ => match game.trap() {
            Some((effect, Trap::Escape(_))) => format!("Trapped by {}: discard a card to try to escape.", lasting_effect_line(&effect).split(':').next().unwrap_or("a trap")),
            Some((_, Trap::PlayScoring)) => "Trapped: only scoring cards can be played this round.".to_string(),
            Some((_, Trap::Skip)) => "Trapped: nothing can be played, so the round is skipped.".to_string(),
            None => match game.status().forced_play {
                Some((forced, _)) if forced == side => "A card is forced into play this round.".to_string(),
                _ => format!("{side} takes an action round."),
            },
        },
    }
}

fn space_line(status: &GameStatus) -> String {
    format!("Space race: US {} · USSR {}", status.space_race_us, status.space_race_ussr)
}

/// The handover modal: `side` is about to act; `lines` are what happened since it last did
/// (already rendered, oldest first), with `earlier` more lines trimmed off the front.
pub fn render_handover(game: &Game, cards: &CardCatalog, side: Superpower, lines: &[String], earlier: usize) -> Canvas {
    let status = game.status();
    let muted = Style::color(Color::Muted);
    let mut out: Vec<(String, Style)> = Vec::new();
    out.push((round_line(game), Style::default().bold()));
    out.push((task_line(game, cards, side), Style::color(side_color(side))));
    out.push((String::new(), Style::default()));

    out.push((format!("DEFCON {} · {}", status.defcon, vp_line(status.vp)), Style::default()));
    let short = status.military_shortfall(side);
    let mil = format!("{side} Military Ops {}/{}", status.military_ops(side), status.defcon);
    if short > 0 {
        out.push((format!("{mil} — {short} VP penalty if the turn ended now"), Style::color(Color::Selected)));
    } else {
        out.push((mil, Style::default()));
    }
    out.push((space_line(status), Style::default()));
    let china = if status.china_card == side {
        if status.china_card_face_up { "the China Card is yours, face up" } else { "the China Card is yours, face down" }
    } else {
        "the China Card is with your opponent"
    };
    out.push((format!("{side} holds {} card{} · {china}", game.hand(side).len(), if game.hand(side).len() == 1 { "" } else { "s" }), Style::default()));

    let effects: Vec<(String, Superpower)> = status
        .lasting
        .active()
        .iter()
        .map(|e| (lasting_effect_line(e), e.side()))
        .chain(status.effects.active().iter().map(|e| (ongoing_effect_line(e), e.side())))
        .collect();
    if !effects.is_empty() {
        out.push((String::new(), Style::default()));
        out.push(("In force:".to_string(), muted));
        for (text, owner) in effects {
            out.push((format!("  {text}"), Style::color(side_color(owner))));
        }
    }

    out.push((String::new(), Style::default()));
    if lines.is_empty() {
        out.push(("Nothing has happened since your last round.".to_string(), muted));
    } else {
        out.push(("Since your last round:".to_string(), muted));
        if earlier > 0 {
            out.push((format!("  … {earlier} earlier"), muted));
        }
        for line in lines {
            out.push((format!("  {line}"), Style::default()));
        }
    }
    out.push((String::new(), Style::default()));
    let text_width = HANDOVER_WIDTH - 6;
    out.push((format!("{:>text_width$}", format!("Pass to {side} · Enter when ready")), muted));
    modal_box(&format!("{side} to play"), "", HANDOVER_WIDTH, Style::color(side_color(side)), true, out)
}
