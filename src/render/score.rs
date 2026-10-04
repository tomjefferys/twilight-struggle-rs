//! The post-event result modal for a resolved scoring card — the Scoring
//! analogue of `roll.rs`'s own post-roll modal. A scoring event is as
//! irreversible as a die roll the instant it resolves (it swings VP, and
//! can end the game outright), so it gets the same titled-box treatment
//! — a per-side tier/battleground/adjacency breakdown, the net VP change,
//! and the new VP total — rather than a dense one-line summary. Meant to
//! be blitted centred over whichever screen `interactive.rs` is showing
//! and dismissed with Enter, same as [`super::render_roll_result`].

use crate::cards::CardCatalog;
use crate::country::Superpower;
use crate::events::scoring::{ScoringKind, SideScore, Tier};
use crate::events::ScoringResult;
use crate::map::WorldMap;

use super::{put_border_title, wrap, Canvas, Color, Style};

/// Fixed content width, like [`super::roll::render_roll_result`]'s own
/// `ROLL_WIDTH` — nothing about this box's width should depend on which
/// card or region is involved.
const SCORE_WIDTH: usize = 56;
/// Columns of padding inside the border on each side.
const PADDING: usize = 2;

/// Draws `result` as a titled box. `queue_pos` carries the same meaning
/// as [`super::roll::render_roll_result`]'s own parameter: `Some((n,
/// total))` once more scoring events (or rolls — the two share one modal
/// queue in `interactive.rs`) are queued behind this one.
pub fn render_scoring_result(
    map: &WorldMap,
    cards: &CardCatalog,
    result: &ScoringResult,
    vp_after: i8,
    queue_pos: Option<(usize, usize)>,
) -> Canvas {
    let hint = match queue_pos {
        Some((n, total)) => format!("Enter to continue · {n} of {total}"),
        None => "Enter to continue".to_string(),
    };
    scoring_box(map, cards, result, vp_after, &hint)
}

/// What playing a scoring card would score right now, before it is played: the same box as
/// [`render_scoring_result`], with the VP track's value it would land on and a confirm/cancel hint.
pub fn render_scoring_preview(map: &WorldMap, cards: &CardCatalog, result: &ScoringResult, vp_after: i8) -> Canvas {
    scoring_box(map, cards, result, vp_after, "Enter play it · Esc cancel")
}

fn scoring_box(map: &WorldMap, cards: &CardCatalog, result: &ScoringResult, vp_after: i8, hint: &str) -> Canvas {
    let title = cards.card(result.card).name.clone();
    let text_width = SCORE_WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();

    match &result.kind {
        ScoringKind::Region { region, us, ussr } => {
            push_side_lines(&mut lines, Superpower::Us, us, text_width);
            lines.push((String::new(), Style::default()));
            push_side_lines(&mut lines, Superpower::Ussr, ussr, text_width);
            lines.push((String::new(), Style::default()));
            push_text(&mut lines, &format!("{region} scoring"), Style::color(Color::Muted), text_width);
            for &card in &result.modifiers {
                push_text(&mut lines, &format!("affected by {}", cards.card(card).name), Style::color(Color::Muted), text_width);
            }
        }
        ScoringKind::SoutheastAsia { controlled } if controlled.is_empty() => {
            push_text(&mut lines, "no Southeast Asia country is controlled", Style::color(Color::Muted), text_width);
        }
        ScoringKind::SoutheastAsia { controlled } => {
            for &(id, side, vp) in controlled {
                lines.push((format!("{}  {side} +{vp}", map.country(id).name), Style::color(side_color(side))));
            }
        }
    }

    lines.push((String::new(), Style::default()));
    let (delta_line, delta_style) = match result.vp_delta.signum() {
        1 => (format!("+{} VP to the US (now {vp_after})", result.vp_delta), Style::color(Color::Us).bold()),
        -1 => (format!("+{} VP to the USSR (now {vp_after})", -result.vp_delta), Style::color(Color::Ussr).bold()),
        _ => (format!("no net VP change (still {vp_after})"), Style::color(Color::Muted).bold()),
    };
    push_text(&mut lines, &delta_line, delta_style, text_width);

    let mut border_color = match result.vp_delta.signum() {
        1 => Color::Us,
        -1 => Color::Ussr,
        _ => Color::Muted,
    };
    if let Some(side) = result.automatic_victory {
        lines.push((String::new(), Style::default()));
        push_text(&mut lines, &format!("{side} WINS THE GAME — Europe Control"), Style::color(side_color(side)).bold(), text_width);
        border_color = side_color(side);
    }

    lines.push((String::new(), Style::default()));
    lines.push((format!("{hint:>text_width$}"), Style::color(Color::Muted)));

    let box_height = 2 + lines.len();
    let mut canvas = Canvas::new(SCORE_WIDTH, box_height);
    canvas.draw_thick_box(0, 0, SCORE_WIDTH, box_height, Style::color(border_color));
    put_border_title(&mut canvas, 0, 0, &title, Style::default().bold(), "", Style::default(), SCORE_WIDTH);

    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }

    canvas
}

/// The headline reveal: both chosen cards side by side with the order they resolve in. `first`
/// resolves first; `cancelled` means Defectors cancels the USSR's.
pub fn render_headline_reveal(
    cards: &CardCatalog,
    ussr: Option<crate::cards::CardId>,
    us: Option<crate::cards::CardId>,
    first: Option<Superpower>,
    cancelled: bool,
    queue_pos: Option<(usize, usize)>,
) -> Canvas {
    let text_width = SCORE_WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();
    let describe = |card: Option<crate::cards::CardId>| match card {
        Some(c) => {
            let c = cards.card(c);
            let ops = if c.scoring { "scoring".to_string() } else { format!("{} ops", c.ops) };
            format!("{} ({ops})", c.name)
        }
        None => "no card to headline".to_string(),
    };
    for (side, card) in [(Superpower::Ussr, ussr), (Superpower::Us, us)] {
        let struck = cancelled && side == Superpower::Ussr;
        let label = if struck { format!("{side:<5} {} — cancelled", describe(card)) } else { format!("{side:<5} {}", describe(card)) };
        push_text(&mut lines, &label, Style::color(side_color(side)).bold(), text_width);
    }
    lines.push((String::new(), Style::default()));
    let order = match (first, cancelled) {
        (Some(side), true) => format!("{side}'s Defectors resolves first and cancels the USSR's headline event"),
        (Some(side), false) => format!("{side} resolves first — the higher Operations value, the US on a tie"),
        (None, _) => "neither side has a headline event".to_string(),
    };
    push_text(&mut lines, &order, Style::color(Color::Muted), text_width);
    lines.push((String::new(), Style::default()));
    let hint = match queue_pos {
        Some((n, total)) => format!("Enter to continue · {n} of {total}"),
        None => "Enter to continue".to_string(),
    };
    lines.push((format!("{hint:>text_width$}"), Style::color(Color::Muted)));
    let height = 2 + lines.len();
    let mut canvas = Canvas::new(SCORE_WIDTH, height);
    canvas.draw_thick_box(0, 0, SCORE_WIDTH, height, Style::color(first.map_or(Color::Muted, side_color)));
    put_border_title(&mut canvas, 0, 0, "Headlines", Style::default().bold(), "", Style::default(), SCORE_WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

/// The confirmation before a headline card is chosen: the first card a side picks in a turn is
/// its headline, played at once as an event, so this gives a chance to take it back.
pub fn render_headline_confirm(side: Superpower, card: &crate::cards::Card) -> Canvas {
    let text_width = SCORE_WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();
    let ops = if card.scoring { "scoring".to_string() } else { format!("{} ops", card.ops) };
    push_text(&mut lines, &format!("{side} headlines {} ({ops})", card.name), Style::color(side_color(side)).bold(), text_width);
    lines.push((String::new(), Style::default()));
    push_text(
        &mut lines,
        "The first card chosen each turn is the headline: only its event is played, before any action round.",
        Style::color(Color::Muted),
        text_width,
    );
    lines.push((String::new(), Style::default()));
    lines.push((format!("{:>text_width$}", "Enter confirm · Esc cancel"), Style::color(Color::Muted)));
    let height = 2 + lines.len();
    let mut canvas = Canvas::new(SCORE_WIDTH, height);
    canvas.draw_thick_box(0, 0, SCORE_WIDTH, height, Style::color(side_color(side)));
    put_border_title(&mut canvas, 0, 0, "Headline card", Style::default().bold(), "", Style::default(), SCORE_WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

/// The summary modal for final scoring after turn 10: each region's swing in the order it
/// was scored, the China Card's point, the VP track at the end and, once decided, the result.
/// `results` pair each region's [`ScoringResult`] with the track after it; `winner` is the
/// game's outcome (`None` while unknown), bordering the box in the winner's colour.
pub fn render_final_scoring(
    results: &[(ScoringResult, i8)],
    china: Option<Superpower>,
    vp_after: i8,
    winner: Option<crate::game::Victory>,
    queue_pos: Option<(usize, usize)>,
) -> Canvas {
    let text_width = SCORE_WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();
    let swing = |delta: i8| match delta.signum() {
        1 => (format!("US +{delta}"), Color::Us),
        -1 => (format!("USSR +{}", -delta), Color::Ussr),
        _ => ("no change".to_string(), Color::Muted),
    };
    for (result, _) in results {
        let name = match &result.kind {
            ScoringKind::Region { region, .. } => region.to_string(),
            ScoringKind::SoutheastAsia { .. } => "Southeast Asia".to_string(),
        };
        let (text, color) = swing(result.vp_delta);
        lines.push((format!("{name:<16} {text}"), Style::color(color)));
        if let Some(side) = result.automatic_victory {
            lines.push((format!("  {side} controls Europe"), Style::color(side_color(side)).bold()));
        }
    }
    if let Some(side) = china {
        lines.push((format!("{:<16} {side} +1", "China Card"), Style::color(side_color(side))));
    }
    lines.push((String::new(), Style::default()));
    let (total, color) = match vp_after.signum() {
        1 => (format!("final: US ahead by {vp_after} VP"), Color::Us),
        -1 => (format!("final: USSR ahead by {} VP", -vp_after), Color::Ussr),
        _ => ("final: level at 0 VP".to_string(), Color::Muted),
    };
    push_text(&mut lines, &total, Style::color(color).bold(), text_width);
    let mut border = color;
    if let Some(victory) = winner {
        push_text(&mut lines, &crate::render::game_over_line(victory), Style::color(victory.side.map_or(Color::Muted, side_color)).bold(), text_width);
        border = victory.side.map_or(Color::Muted, side_color);
    }
    lines.push((String::new(), Style::default()));
    let hint = match queue_pos {
        Some((n, total)) => format!("Enter to continue · {n} of {total}"),
        None => "Enter to continue".to_string(),
    };
    lines.push((format!("{hint:>text_width$}"), Style::color(Color::Muted)));

    let height = 2 + lines.len();
    let mut canvas = Canvas::new(SCORE_WIDTH, height);
    canvas.draw_thick_box(0, 0, SCORE_WIDTH, height, Style::color(border));
    put_border_title(&mut canvas, 0, 0, "Final scoring", Style::default().bold(), "", Style::default(), SCORE_WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

/// One side's own breakdown: a bold header line (tier, country/
/// battleground counts) followed by an indented, word-wrapped line
/// itemising the VP contributions that summed to its own total — the
/// same "wrap rather than widen the box" rule
/// [`super::roll::render_roll_result`]'s modifier breakdown follows.
fn push_side_lines(lines: &mut Vec<(String, Style)>, side: Superpower, score: &SideScore, text_width: usize) {
    let header = format!("{side}  {}  ({} countries, {} battlegrounds)", tier_label(score.tier), score.countries, score.battlegrounds);
    lines.push((header, Style::color(side_color(side)).bold()));

    let mut parts = Vec::new();
    if score.tier_vp > 0 {
        parts.push(format!("{} {}", score.tier_vp, tier_label(score.tier)));
    }
    if score.battleground_vp > 0 {
        parts.push(format!("{} battleground", score.battleground_vp));
    }
    if score.adjacency_vp > 0 {
        parts.push(format!("{} adjacency", score.adjacency_vp));
    }
    let detail =
        if parts.is_empty() { "0 VP".to_string() } else { format!("{} = {} VP", parts.join(" + "), score.total()) };
    for line in wrap(&detail, text_width.saturating_sub(2)) {
        lines.push((format!("  {line}"), Style::color(Color::Muted)));
    }
}

fn push_text(lines: &mut Vec<(String, Style)>, text: &str, style: Style, text_width: usize) {
    for line in wrap(text, text_width) {
        lines.push((line, style));
    }
}

fn tier_label(tier: Tier) -> &'static str {
    match tier {
        Tier::None => "no tier",
        Tier::Presence => "presence",
        Tier::Domination => "domination",
        Tier::Control => "control",
    }
}

fn side_color(side: Superpower) -> Color {
    match side {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::Region;
    use crate::country::Superpower::*;
    use crate::render::ColorMode;

    fn cards() -> CardCatalog {
        CardCatalog::standard().unwrap()
    }

    fn map() -> WorldMap {
        WorldMap::standard().unwrap()
    }

    fn side_score(tier: Tier, countries: u8, battlegrounds: u8, tier_vp: u8, battleground_vp: u8, adjacency_vp: u8) -> SideScore {
        SideScore { tier, countries, battlegrounds, tier_vp, battleground_vp, adjacency_vp }
    }

    #[test]
    fn a_region_result_names_both_sides_tiers_and_the_net_vp() {
        let cards = cards();
        let map = map();
        let card = cards.id_by_name("Middle East Scoring").unwrap();
        let us = side_score(Tier::Presence, 1, 0, 3, 0, 0);
        let ussr = side_score(Tier::Domination, 3, 1, 5, 1, 0);
        let result = ScoringResult {
            card,
            kind: ScoringKind::Region { region: Region::MiddleEast, us, ussr },
            vp_delta: 3 - (5 + 1),
            automatic_victory: None, modifiers: vec![]
        };
        let canvas = render_scoring_result(&map, &cards, &result, -3, None);
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("Middle East Scoring"), "{text}");
        assert!(text.contains("USA  presence"), "{text}");
        assert!(text.contains("USSR  domination"), "{text}");
        assert!(text.contains("+3 VP to the USSR (now -3)"), "{text}");
        for line in text.lines() {
            assert!(line.chars().count() <= SCORE_WIDTH, "line too wide: {line:?}");
        }
    }

    #[test]
    fn a_tied_region_result_shows_no_net_change() {
        let cards = cards();
        let map = map();
        let card = cards.id_by_name("Africa Scoring").unwrap();
        let score = side_score(Tier::None, 0, 0, 0, 0, 0);
        let result = ScoringResult {
            card,
            kind: ScoringKind::Region { region: Region::Africa, us: score, ussr: score },
            vp_delta: 0,
            automatic_victory: None, modifiers: vec![]
        };
        let canvas = render_scoring_result(&map, &cards, &result, 0, None);
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("no net VP change (still 0)"), "{text}");
    }

    #[test]
    fn europe_control_names_the_winner_and_borders_in_their_colour() {
        let cards = cards();
        let map = map();
        let card = cards.id_by_name("Europe Scoring").unwrap();
        let us = side_score(Tier::None, 0, 0, 0, 0, 0);
        let ussr = side_score(Tier::Control, 6, 5, 0, 5, 0);
        let result = ScoringResult {
            card,
            kind: ScoringKind::Region { region: Region::Europe, us, ussr },
            vp_delta: -5,
            automatic_victory: Some(Ussr), modifiers: vec![]
        };
        let canvas = render_scoring_result(&map, &cards, &result, -5, None);
        let text = canvas.render(ColorMode::Always);
        let plain = canvas.render(ColorMode::Never);
        assert!(plain.contains("USSR WINS THE GAME"), "{plain}");
        assert!(text.contains("\x1b[1;91m"), "the box should border bold red for a USSR win: {text:?}");
    }

    #[test]
    fn southeast_asia_lists_each_controlled_country() {
        let cards = cards();
        let map = map();
        let card = cards.id_by_name("Southeast Asia Scoring").unwrap();
        let thailand = map.id_by_name("Thailand").unwrap();
        let vietnam = map.id_by_name("Vietnam").unwrap();
        let result = ScoringResult {
            card,
            kind: ScoringKind::SoutheastAsia { controlled: vec![(thailand, Ussr, 2), (vietnam, Us, 1)] },
            vp_delta: 1 - 2,
            automatic_victory: None, modifiers: vec![]
        };
        let canvas = render_scoring_result(&map, &cards, &result, -1, Some((2, 3)));
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("Thailand  USSR +2"), "{text}");
        assert!(text.contains("Vietnam  USA +1"), "{text}");
        assert!(text.contains("2 of 3"), "{text}");
    }

    #[test]
    fn southeast_asia_with_nothing_controlled_says_so() {
        let cards = cards();
        let map = map();
        let card = cards.id_by_name("Southeast Asia Scoring").unwrap();
        let result =
            ScoringResult { card, kind: ScoringKind::SoutheastAsia { controlled: vec![] }, vp_delta: 0, automatic_victory: None, modifiers: vec![] };
        let canvas = render_scoring_result(&map, &cards, &result, 0, None);
        let text = canvas.render(ColorMode::Never);
        assert!(text.contains("no Southeast Asia country is controlled"), "{text}");
    }

    /// Every real scoring card's name should fit the box via its own
    /// title border, the same width guarantee `card.rs` tests pin for
    /// every catalog entry.
    #[test]
    fn every_scoring_card_name_fits_the_box() {
        let cards = cards();
        let map = map();
        let score = side_score(Tier::None, 0, 0, 0, 0, 0);
        for id in [1u8, 2, 3, 37, 38, 79, 81] {
            let card = cards.find(&id.to_string());
            let crate::cards::CardFound::One(card) = card else { panic!("card #{id} should resolve uniquely") };
            let result = match cards.card(card).name.as_str() {
                "Southeast Asia Scoring" => {
                    ScoringResult { card, kind: ScoringKind::SoutheastAsia { controlled: vec![] }, vp_delta: 0, automatic_victory: None, modifiers: vec![] }
                }
                _ => ScoringResult {
                    card,
                    kind: ScoringKind::Region { region: Region::Europe, us: score, ussr: score },
                    vp_delta: 0,
                    automatic_victory: None, modifiers: vec![]
                },
            };
            let canvas = render_scoring_result(&map, &cards, &result, 0, None);
            for line in canvas.render(ColorMode::Never).lines() {
                assert!(line.chars().count() <= SCORE_WIDTH, "card #{id}: line too wide: {line:?}");
            }
        }
    }
}
