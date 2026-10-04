//! Turning a [`GameLog`] into text: one line per entry, fixed columns, no
//! wrapping. This is the single source of truth for both what `log` prints
//! in the REPL and what `export` writes to a file — [`log_text`] is what
//! [`render_log`] draws, character for character, so the two can't drift
//! apart into two formats.
//!
//! Every number a roll produces is tagged with where it came from (`d6:`
//! for the actual die, `mod:`/`ops:` for what board state or the card
//! contributed, `sum:` for the modified total) rather than shown as a bare
//! sum — a `4+4=8` reads as ambiguous once both the die and the ops value
//! can be 4.

use crate::cards::{CardCatalog, CardId};
use crate::country::{CountryId, Superpower};
use crate::events::scoring::{ScoringKind, SideScore, Tier};
use crate::events::{EffectResult, ScoringResult};
use crate::game::{OperationKind, Victory, VictoryReason};
use crate::log::{CoupAftermath, Event, GameLog, LogEntry};
use crate::map::WorldMap;
use crate::ops::{CoupResult, RollResult};

use super::{Canvas, Color, Style};

const STAMP_WIDTH: usize = 4;
const AR_WIDTH: usize = 5;
const SIDE_WIDTH: usize = 5;
const ACTION_WIDTH: usize = 11;

/// The canonical text of one entry, with no trailing whitespace — matching
/// what [`Canvas::render`] would emit for the same row, since both trim
/// trailing spaces the same way.
pub fn log_entry_line(map: &WorldMap, cards: &CardCatalog, entry: &LogEntry) -> String {
    let (action, detail) = action_and_detail(map, cards, entry);
    let line = format!(
        "{:<STAMP_WIDTH$}{:<AR_WIDTH$}{:<SIDE_WIDTH$}{:<ACTION_WIDTH$}{detail}",
        format!("T{}", entry.turn),
        format!("AR{}", entry.action_round),
        side_label(entry.side),
        action,
    );
    line.trim_end().to_string()
}

/// The whole log as plain text: two `#`-prefixed header lines (so a parser
/// can skip them, and a reader gets a legend), then one [`log_entry_line`]
/// per entry. What `export` writes.
pub fn log_text(map: &WorldMap, cards: &CardCatalog, log: &GameLog) -> String {
    header_lines()
        .into_iter()
        .chain(log.entries().iter().map(|e| log_entry_line(map, cards, e)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The log as a styled [`Canvas`]: each entry's stamp and action bold in
/// its side's colour, the detail in that colour unbolded — `None` (a debug
/// edit or a note) renders wholly [`Color::Muted`]. `tail`, if given, shows
/// only the last `n` entries.
///
/// `render_log(..).render(ColorMode::Never)` is guaranteed to equal
/// [`log_text`] for the same log and the same `tail` — colour never
/// changes what character ends up in a cell, only how it's painted.
pub fn render_log(map: &WorldMap, cards: &CardCatalog, log: &GameLog, tail: Option<usize>) -> Canvas {
    let entries = match tail {
        Some(n) => log.tail(n),
        None => log.entries(),
    };
    let header = header_lines();
    let lines: Vec<String> = header.iter().cloned().chain(entries.iter().map(|e| log_entry_line(map, cards, e))).collect();
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(1);
    let mut canvas = Canvas::new(width, lines.len());

    for (row, line) in header.iter().enumerate() {
        canvas.put(row, 0, line, Style::color(Color::Muted));
    }
    for (i, entry) in entries.iter().enumerate() {
        let row = header.len() + i;
        let color = match entry.side {
            Some(Superpower::Us) => Color::Us,
            Some(Superpower::Ussr) => Color::Ussr,
            None => Color::Muted,
        };
        let (action, detail) = action_and_detail(map, cards, entry);
        let prefix = format!(
            "{:<STAMP_WIDTH$}{:<AR_WIDTH$}{:<SIDE_WIDTH$}{:<ACTION_WIDTH$}",
            format!("T{}", entry.turn),
            format!("AR{}", entry.action_round),
            side_label(entry.side),
            action,
        );
        let prefix_len = prefix.chars().count();
        canvas.put(row, 0, &prefix, Style::color(color).bold());
        canvas.put(row, prefix_len, &detail, Style::color(color));
    }
    canvas
}

fn header_lines() -> Vec<String> {
    vec!["# twilight-struggle game log".to_string(), "# turn ar   side action   detail".to_string()]
}

fn side_label(side: Option<Superpower>) -> String {
    side.map(|s| s.to_string()).unwrap_or_else(|| "----".to_string())
}

/// The action keyword and the detail text for one entry. Split out from
/// [`log_entry_line`] so [`render_log`] can style the two parts
/// differently without duplicating the match.
fn action_and_detail(map: &WorldMap, cards: &CardCatalog, entry: &LogEntry) -> (&'static str, String) {
    match &entry.event {
        // The first thing a turn does — logged, once irrevocable, before
        // the operation it funds has necessarily closed (see
        // `Game::log_card_selected`'s own doc for exactly when).
        Event::Selected { card } => ("card", selected_detail(cards, *card)),
        // The placement analogue of a resolved roll: its own "influence"
        // line, pushed just before the `Closed` entry that reports whether
        // it was confirmed or cancelled.
        Event::Placed { countries } => ("influence", placed_detail(map, countries)),
        Event::Realign(result) => {
            let side = entry.side.expect("a realignment roll is always stamped with the acting side");
            ("realign", realign_detail(map, side, result))
        }
        Event::Coup(result) => {
            let side = entry.side.expect("a coup attempt is always stamped with the acting side");
            ("coup", coup_detail(map, side, result))
        }
        Event::CoupAftermath(aftermath) => ("aftermath", aftermath_detail(aftermath)),
        Event::Closed { kind, committed, rolls, ops_spent, ops_total } => {
            let action = if *committed { "confirm" } else { "cancel" };
            (action, closed_detail(*kind, *rolls, *ops_spent, *ops_total))
        }
        Event::Triggered { card, vp_delta, vp_after } => {
            ("trigger", format!("{} {vp_delta:+} VP (now {vp_after})", cards.card(*card).name))
        }
        Event::Pass => ("pass", String::new()),
        Event::Edit { country, side, before, after } => ("edit", edit_detail(map, *country, *side, *before, *after)),
        Event::Note(text) => ("note", text.clone()),
        Event::Scored { result, vp_after } => ("score", scored_detail(map, cards, result, *vp_after)),
        Event::EventResolved { result, vp_after } => ("event", event_detail(map, cards, result, *vp_after)),
        Event::War { result, vp_after } => ("war", war_detail(map, cards, result, *vp_after)),
        Event::Space { result, vp_after } => ("space", space_detail(cards, result, *vp_after)),
        Event::GameOver(victory) => ("gameover", game_over_detail(*victory)),
    }
}

fn ops_str(spent: u8, total: u8) -> String {
    if spent == total {
        format!("{spent} ops")
    } else {
        format!("{spent} of {total} ops")
    }
}

/// The card named, with its ops value — e.g. `Fidel (2 ops)`.
fn selected_detail(cards: &CardCatalog, card: CardId) -> String {
    let card = cards.card(card);
    format!("{} ({} ops)", card.name, card.ops)
}

/// `Event::Placed` is only ever pushed with at least one country (see its
/// own doc), so there's no empty case to render here.
fn placed_detail(map: &WorldMap, countries: &[(CountryId, u8)]) -> String {
    countries.iter().map(|&(id, n)| format!("{} +{n}", map.country(id).name)).collect::<Vec<_>>().join(", ")
}

fn closed_detail(kind: OperationKind, rolls: u8, spent: u8, total: u8) -> String {
    let ops = ops_str(spent, total);
    match kind {
        OperationKind::Realign => {
            let noun = if rolls == 1 { "roll" } else { "rolls" };
            format!("realign, {rolls} {noun}, {ops}")
        }
        OperationKind::Coup => format!("coup, {ops}"),
        OperationKind::Influence => format!("influence, {ops}"),
    }
}

/// Every number tagged with where it came from: `d6:` is only ever the
/// actual die, `mod:` is the board-derived modifier total, `sum:` is the
/// two added together — so a reader (or a parser) never has to guess which
/// number in `4+1=5` was rolled and which came from the board.
fn realign_detail(map: &WorldMap, side: Superpower, result: &RollResult) -> String {
    let opponent = side.opponent();
    let country = &map.country(result.target).name;
    let acting_sum = result.acting_die as i16 + result.acting_mods.total() as i16;
    let opposing_sum = result.opposing_die as i16 + result.opposing_mods.total() as i16;
    let outcome = match result.loser {
        None => "tie".to_string(),
        Some(loser) => format!("{loser} -{}", result.removed),
    };
    format!(
        "{country}  {side} d6:{} mod:{:+} sum:{acting_sum}  vs  {opponent} d6:{} mod:{:+} sum:{opposing_sum}  -> {outcome}",
        result.acting_die,
        result.acting_mods.total(),
        result.opposing_die,
        result.opposing_mods.total(),
    )
}

/// Same tagging discipline as [`realign_detail`]: `d6:` the die, `ops:`
/// the card's operation points, `target:` the number to beat (with its
/// `stability x2` derivation spelled out), `sum:` the two dice/ops added.
fn coup_detail(map: &WorldMap, side: Superpower, result: &CoupResult) -> String {
    let opponent = side.opponent();
    let country = &map.country(result.target).name;
    let stability = map.country(result.target).stability;
    let sum = result.die as i16 + result.ops as i16 + result.modifier as i16;
    let modifier = if result.modifier == 0 { String::new() } else { format!(" mod:{:+}", result.modifier) };
    let outcome = if !result.success() {
        "failed".to_string()
    } else {
        match (result.removed, result.added) {
            (removed, 0) => format!("success by {}: {opponent} -{removed}", result.margin),
            (0, added) => format!("success by {}: {side} +{added}", result.margin),
            (removed, added) => format!("success by {}: {opponent} -{removed}, {side} +{added}", result.margin),
        }
    };
    format!(
        "{country}  d6:{} ops:+{}{modifier} sum:{sum}  vs  target:{} (stability:{stability} x2)  -> {outcome}",
        result.die, result.ops, result.target_number,
    )
}

/// `DEFCON 4→3, +1 VP (now -2)` — whatever a coup set off besides its own
/// board result.
fn aftermath_detail(aftermath: &CoupAftermath) -> String {
    let mut parts = Vec::new();
    if let Some((before, after)) = aftermath.defcon {
        parts.push(format!("DEFCON {before}→{after}"));
    }
    if aftermath.defcon_spared {
        parts.push("Nuclear Subs: DEFCON unchanged".to_string());
    }
    if let Some((delta, vp_after)) = aftermath.vp {
        parts.push(format!("{delta:+} VP (now {vp_after})"));
    }
    parts.join(", ")
}

fn edit_detail(map: &WorldMap, country: CountryId, side: Superpower, before: u8, after: u8) -> String {
    format!("{} {side} {before} -> {after}", map.country(country).name)
}

fn tier_label(tier: Tier) -> &'static str {
    match tier {
        Tier::None => "none",
        Tier::Presence => "pres",
        Tier::Domination => "dom",
        Tier::Control => "ctrl",
    }
}

/// One side's own slice of a region scoring line — e.g. `US 7 (dom, 2 BG,
/// 1 adj)` — omitting the battleground/adjacency parentheticals whenever
/// they're zero, the same "don't show a contribution that didn't apply"
/// rule `realign::Modifiers::reasons` follows.
fn side_breakdown(side: Superpower, score: &SideScore) -> String {
    let mut parts = vec![tier_label(score.tier).to_string()];
    if score.battleground_vp > 0 {
        parts.push(format!("{} BG", score.battleground_vp));
    }
    if score.adjacency_vp > 0 {
        parts.push(format!("{} adj", score.adjacency_vp));
    }
    format!("{side} {} ({})", score.total(), parts.join(", "))
}

/// `result.vp_delta` and `vp_after` are both carried on the `Scored`
/// entry itself (see its own doc) rather than recomputed here — this
/// only has to format them.
fn scored_detail(map: &WorldMap, cards: &CardCatalog, result: &ScoringResult, vp_after: i8) -> String {
    let name = &cards.card(result.card).name;
    let body = match &result.kind {
        ScoringKind::Region { region, us, ussr } => {
            format!("{region}: {} · {}", side_breakdown(Superpower::Us, us), side_breakdown(Superpower::Ussr, ussr))
        }
        ScoringKind::SoutheastAsia { controlled } if controlled.is_empty() => "no countries controlled".to_string(),
        ScoringKind::SoutheastAsia { controlled } => controlled
            .iter()
            .map(|&(id, side, vp)| format!("{} {side}+{vp}", map.country(id).name))
            .collect::<Vec<_>>()
            .join(", "),
    };
    let modifiers: String = result.modifiers.iter().map(|&c| format!(" [{}]", cards.card(c).name)).collect();
    format!("{name}: {body}{modifiers} -> {:+} VP (now {vp_after})", result.vp_delta)
}

/// `Fidel: Cuba US 1→0, Cuba USSR 0→3 · +2 VP (now 5)` — only the parts
/// the card actually changed, so a VP-only card is just its VP.
fn event_detail(map: &WorldMap, cards: &CardCatalog, result: &EffectResult, vp_after: i8) -> String {
    let mut parts: Vec<String> = result
        .influence
        .iter()
        .map(|c| format!("{} {} {}→{}", map.country(c.country).name, c.side, c.before, c.after))
        .collect();
    if let Some((before, after)) = result.defcon {
        parts.push(format!("DEFCON {before}→{after}"));
    }
    if result.mil_ops != 0 {
        parts.push(format!("{} Mil Ops {:+}", result.player, result.mil_ops));
    }
    if let Some((side, card)) = result.discards {
        parts.push(format!("{side} discards {}", cards.card(card).name));
    }
    if let Some(reveal) = &result.reveals {
        let names: Vec<&str> = reveal.cards.iter().map(|&c| cards.card(c).name.as_str()).collect();
        parts.push(format!("{} hand: {}", reveal.side, if names.is_empty() { "(empty)".to_string() } else { names.join(", ") }));
    }
    if let Some(t) = &result.china {
        parts.push(format!("China Card→{} ({})", t.to, if t.face_up { "face up" } else { "face down" }));
    }
    if let Some((side, from, to)) = result.space {
        parts.push(format!("{side} space race {from}→{to} ({})", crate::space::space_box(to).name));
    }
    if result.vp_delta != 0 {
        parts.push(format!("{:+} VP (now {vp_after})", result.vp_delta));
    }
    if let Some(effect) = &result.ongoing {
        let line = super::ongoing_effect_line(effect);
        parts.push(format!("this turn: {}", line.split_once(": ").map_or(line.as_str(), |(_, rest)| rest)));
    }
    let body = if parts.is_empty() { "no effect".to_string() } else { parts.join(", ") };
    format!("{}: {body}", cards.card(result.card).name)
}

/// `Duck and Cover → Animal in Space: d6:2 need:≤4 — box 2, +0 VP (now -3)`,
/// tagged like every other roll line.
fn space_detail(cards: &CardCatalog, result: &crate::space::SpaceResult, vp_after: i8) -> String {
    let target = result.target();
    let outcome = if result.success {
        let vp = if result.vp_delta != 0 { format!(", {:+} VP (now {vp_after})", result.vp_delta) } else { String::new() };
        format!("reaches box {}{vp}", result.from + 1)
    } else {
        format!("stays at box {}", result.from)
    };
    format!("{} → {}: d6:{} need:≤{} — {outcome}", cards.card(result.card).name, target.name, result.roll, result.max_roll)
}

/// `Korean War → South Korea: d6:4 mod:-1 need:4+ → fails ...`, tagged like
/// every other roll line.
fn war_detail(map: &WorldMap, cards: &CardCatalog, result: &crate::events::WarResult, vp_after: i8) -> String {
    let mut parts = vec![format!(
        "{} → {}: {} d6:{} mod:{:+} sum:{} need:{}+ — {}",
        cards.card(result.card).name,
        map.country(result.target).name,
        result.side,
        result.die,
        result.modifier.total(),
        result.modified(),
        result.success_min,
        if result.success { "WINS" } else { "fails" },
    )];
    for c in &result.influence {
        parts.push(format!("{} {} {}→{}", map.country(c.country).name, c.side, c.before, c.after));
    }
    if result.vp_delta != 0 {
        parts.push(format!("{:+} VP (now {vp_after})", result.vp_delta));
    }
    parts.push(format!("{} mil ops +{}", result.side, result.mil_ops));
    parts.join(", ")
}

fn game_over_detail(victory: Victory) -> String {
    let reason = match victory.reason {
        VictoryReason::Vp => "VP",
        VictoryReason::EuropeControl => "Europe control",
        VictoryReason::Defcon => "DEFCON 1",
        VictoryReason::Wargames => "Wargames",
    };
    format!("{} wins ({reason})", victory.side)
}
