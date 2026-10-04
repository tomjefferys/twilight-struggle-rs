//! The two-row turn/operation bar `interactive.rs` draws above every map
//! screen (world, region, country) — the one thing that makes whose turn
//! it is, and what's currently in progress, unmistakable from a keypress
//! alone. A pure [`Canvas`] producer like every other view; nothing here
//! touches a terminal.

use crate::board::Board;
use crate::cards::Card;
use crate::country::Superpower;
use crate::events;
use crate::game::Victory;
use crate::layout::MapLayout;
use crate::ops::Operation;
use crate::space::{self, Perk};
use crate::status::GameStatus;

use super::{game_over_line, lasting_effect_line, ongoing_effect_line, operation_balance_line, vp_line, Canvas, Color, Style};
use crate::ongoing::short_name;

/// Row 1's wording when no card is in play yet — the status bar's own
/// three-state hint (see [`render_status_bar`]'s own doc), distinct from
/// `BEGIN_HINT`'s card-agnostic one-liner shown by the region/world-map/
/// country views, which never know whether a card's already been played.
const PLAY_HINT: &str = "no card in play — [ ] select · space play · p pass";

/// The bar's height, always — with or without an operation open — so the
/// view drawn below it never shifts up or down as one opens or closes.
pub const STATUS_BAR_ROWS: usize = 4;

/// Row 0: turn/AR/active side/DEFCON/VP. Row 1 is one of four states,
/// depending on `winner`, `card` (the card currently in play, if any —
/// see [`crate::game::Game::card_in_play`]), and `op`:
/// - A winner ([`crate::game::Game::winner`]): `GAME OVER — <side> wins
///   (<reason>)`, in that side's own colour — replacing the card/operation
///   row entirely, since there's nothing left to play.
/// - No card in play: [`PLAY_HINT`], naming the keys to select and play
///   one.
/// - A card in play, no operation open yet: the card's own name, with the
///   keys to spend it or return it — `e score` in place of the ops keys
///   for a scoring card (it has none to spend), its own ops/keys
///   otherwise.
/// - An operation open: the exact text `operation_balance_line` gives the
///   region and world-map footers (so the three can't disagree), prefixed
///   with the card's name — the only one of the three states that also
///   needed `board`.
///
/// Row 2: the turn-long card effects in force (see [`crate::ongoing`]),
/// each in its beneficiary's colour — or a muted note that there are none,
/// so the row (and the bar's height) never changes.
///
/// Row 3: a plain rule, separating the bar from whichever screen it sits
/// above.
///
/// `board` should be the caller's *committed* board, not a speculative
/// one — `operation_balance_line` only reads it through a realignment's or
/// coup's `delta`, which always tracks the real board regardless (a
/// placement's own pending points come from `op` itself).
///
/// `width` is a minimum, not a fixed size: the canvas grows to fit
/// whichever row is longer, the same rule `render_world`/`render_region`
/// already follow for their own content, so nothing here is ever clipped.
pub fn render_status_bar(
    layout: &MapLayout,
    board: &Board,
    status: &GameStatus,
    card: Option<&Card>,
    op: Option<&Operation>,
    winner: Option<Victory>,
    width: usize,
) -> Canvas {
    render_status_bar_with(layout, board, status, card, op, winner, None, width)
}

/// [`render_status_bar`] for a game whose card in play has already had its
/// event resolved and is waiting for the operation `after_event` allows
/// ([`crate::game::Game::ops_after_event`]).
#[allow(clippy::too_many_arguments)]
pub fn render_status_bar_with(
    layout: &MapLayout,
    board: &Board,
    status: &GameStatus,
    card: Option<&Card>,
    op: Option<&Operation>,
    winner: Option<Victory>,
    after_event: Option<crate::events::OpsGrant>,
    width: usize,
) -> Canvas {
    // An open event's chooser (the card's own side) is who's really to
    // act, even when the other side is phasing.
    let to_act = match op {
        Some(Operation::Event(e)) => e.chooser(),
        _ => status.active,
    };
    let turn_line = format!(
        "TURN {} · AR {} · {} to act · DEFCON {} · VP {}{}",
        status.turn,
        ar_label(status),
        to_act,
        status.defcon,
        vp_line(status.vp),
        space_label(status),
    );
    let (op_line, op_style) = if let Some(victory) = winner {
        (game_over_line(victory), side_style(victory.side).bold())
    } else {
        match (card, op) {
            // A discard-or-suffer decision gets its own compact line: the keys
            // first and the (long) consequence of the current choice last, so a
            // narrow terminal clips the least important part.
            (_, Some(Operation::Event(e))) if !e.gate_cards().is_empty() => {
                let name = card.map(|c| c.name.as_str()).unwrap_or("?");
                let line = match e.mode() {
                    None if e.gate_offset() == 0 => {
                        format!("{name} · {} {} · [ ] pick a card · space discard it · then c", e.chooser(), e.gate_prompt())
                    }
                    None => format!("{name} · {} {} · [ ] pick a card · space discard it · 1 keep your cards · then c", e.chooser(), e.gate_prompt()),
                    Some(i) => format!("{name} · {} · c confirm · ⌫ undo · [ ] space change → {}", e.chooser(), e.modes()[i].label),
                };
                (line, side_style(e.chooser()).bold())
            }
            (_, Some(operation @ Operation::Event(e))) => {
                let name = card.map(|c| c.name.as_str()).unwrap_or(if e.is_triggered() { "NORAD" } else { "?" });
                let n = e.modes().len();
                let keys = match (e.mode(), e.is_designation()) {
                    _ if e.needs_roll() && e.is_participation() => format!("r roll the dice · 1-{n} change · ⌫ clear"),
                    _ if e.needs_roll() => "r roll the dice · ⌫ cancel the event".to_string(),
                    (None, false) if !e.gate_cards().is_empty() => "[ ] pick a card · space discard it · 1 keep your cards".to_string(),
                    (Some(_), false) if !e.gate_cards().is_empty() => "[ ] pick a card · space discard it · 1 keep · c done".to_string(),
                    (None, true) => format!("Enter on world map or 1-{n} to choose region"),
                    (None, false) => format!("1-{n} choose mode"),
                    (Some(_), true) => format!("Enter/1-{n} change region · ⌫ clear · c done"),
                    (Some(_), false) if !e.picks_countries() => format!("1-{n} change · c done"),
                    (Some(_), false) => "+ add · - remove · u undo · c done".to_string(),
                };
                // A choice settled by its mode alone has a long prompt (the roll-off, the options):
                // keep its keys in front where a narrow terminal won't clip them.
                let line = if e.is_mode_only() {
                    format!("{name} · {keys} · {}", operation_balance_line(layout, board, operation))
                } else {
                    format!("{name} · {} · {keys}", operation_balance_line(layout, board, operation))
                };
                (line, side_style(e.chooser()).bold())
            }
            (_, Some(operation)) => {
                let name = card.map(|c| c.name.as_str()).unwrap_or("?");
                (format!("{name} · {}", operation_balance_line(layout, board, operation)), Style::color(Color::Selected))
            }
            (Some(card), None) if after_event.is_some() => {
                let grant = after_event.expect("guard checked");
                let keys: Vec<&str> = [(grant.influence, "i influence"), (grant.realign, "a realign"), (grant.coup, "o coup")]
                    .into_iter()
                    .filter_map(|(on, k)| on.then_some(k))
                    .collect();
                (
                    format!(
                        "{} event played ({}) — {} · p skip the ops",
                        card.name,
                        grant.ops.map_or_else(|| ops_text(status, card), |o| format!("{} ops", status.effects.card_ops(o, status.active).0)),
                        keys.join(" · ")
                    ),
                    Style::color(Color::Selected),
                )
            }
            (Some(card), None) if card.scoring => {
                (format!("playing {} — e score · ⌫ return card", card.name), Style::color(Color::Selected))
            }
            (Some(card), None) if events::is_implemented(card.id) => (
                format!("playing {} ({}) — e event · i influence · a realign · o coup{} · ⌫ return card", card.name, ops_text(status, card), space_hint(card)),
                Style::color(Color::Selected),
            ),
            (Some(card), None) => (
                format!("playing {} ({}) — i influence · a realign · o coup{} · ⌫ return card", card.name, ops_text(status, card), space_hint(card)),
                Style::color(Color::Selected),
            ),
            (None, None) => match status.lasting.trap_on(status.active) {
                Some(trap) => (
                    format!(
                        "{} traps {} · space discard a 2+ ops card and roll (1-4 escapes) · no such card: play scoring cards, then p",
                        trap.label(),
                        status.active
                    ),
                    side_style(status.active).bold(),
                ),
                None => (PLAY_HINT.to_string(), Style::color(Color::Muted)),
            },
        }
    };

    // Game-long effects first, then the turn-long ones, each in its beneficiary's colour.
    let effects: Vec<(String, Superpower)> = status
        .lasting
        .active()
        .iter()
        .map(|e| (lasting_effect_line(e), e.side()))
        .chain(status.effects.active().iter().map(|e| (ongoing_effect_line(e), e.side())))
        .chain(Perk::ALL.iter().filter_map(|&p| space::perk_holder(status, p).map(|side| (format!("{side} space: {}", p.label()), side))))
        .collect();
    let effects_width = if effects.is_empty() {
        NO_EFFECTS.chars().count()
    } else {
        EFFECTS_LABEL.chars().count() + effects.iter().map(|(t, _)| t.chars().count()).sum::<usize>() + EFFECT_SEP.chars().count() * (effects.len() - 1)
    };
    let content_width = width.max(turn_line.chars().count()).max(op_line.chars().count()).max(effects_width).max(1);
    let mut canvas = Canvas::new(content_width, STATUS_BAR_ROWS);

    draw_turn_line(&mut canvas, status, to_act);
    canvas.put(1, 0, &op_line, op_style);
    draw_effects_line(&mut canvas, &effects);
    canvas.put(3, 0, &"─".repeat(content_width), Style::color(Color::Muted));

    canvas
}

const NO_EFFECTS: &str = "no turn effects in force";
const EFFECTS_LABEL: &str = "In effect: ";
const EFFECT_SEP: &str = " · ";

/// ` · Space 2-1` — the USA's box, then the USSR's.
fn space_label(status: &GameStatus) -> String {
    format!(" · Space {}-{}", status.space_race_us, status.space_race_ussr)
}

/// ` · s space race` for any card that has ops to spend — the key opens a
/// confirmation that explains the box, and why a roll isn't available if it isn't.
fn space_hint(card: &Card) -> &'static str {
    if card.scoring { "" } else { " · s space race" }
}

/// `3`, or `8/7+1` for the extra round North Sea Oil gives the US.
fn ar_label(status: &GameStatus) -> String {
    let (round, per_turn) = (status.action_round, status.action_rounds_per_turn);
    if round > per_turn { format!("{round}/{per_turn}+{}", round - per_turn) } else { format!("{round}/{per_turn}") }
}

/// A card's ops as the active side would actually spend them, itemised
/// when an event changes them — `3 ops: 2 +1 Brezhnev` — and noting a
/// sub-region bonus on offer (Vietnam Revolts).
fn ops_text(status: &GameStatus, card: &Card) -> String {
    let (ops, mods) = status.effects.card_ops(card.ops, status.active);
    let mut text = format!("{ops} ops");
    if !mods.is_empty() {
        let parts: Vec<String> = mods.iter().map(|&(id, m)| format!("{m:+} {}", short_name(id))).collect();
        text.push_str(&format!(": {} {}", card.ops, parts.join(" ")));
    }
    for b in status.effects.ops_bonuses(status.active, card.id) {
        text.push_str(&format!(", +{} if all in {}", b.ops, b.area));
    }
    text
}

/// Row 2, drawn piecewise so each effect carries its beneficiary's colour.
fn draw_effects_line(canvas: &mut Canvas, effects: &[(String, Superpower)]) {
    if effects.is_empty() {
        canvas.put(2, 0, NO_EFFECTS, Style::color(Color::Muted));
        return;
    }
    let mut col = 0;
    canvas.put(2, col, EFFECTS_LABEL, Style::default());
    col += EFFECTS_LABEL.chars().count();
    for (i, (text, side)) in effects.iter().enumerate() {
        if i > 0 {
            canvas.put(2, col, EFFECT_SEP, Style::color(Color::Muted));
            col += EFFECT_SEP.chars().count();
        }
        canvas.put(2, col, text, side_style(*side));
        col += text.chars().count();
    }
}

fn side_style(side: Superpower) -> Style {
    match side {
        Superpower::Us => Style::color(Color::Us),
        Superpower::Ussr => Style::color(Color::Ussr),
    }
}

/// Row 0, drawn as several `put` calls rather than one string so the
/// active side's own name can carry its own colour (`Color::Us`/`Ussr`)
/// and bold weight while the rest of the row stays plain.
fn draw_turn_line(canvas: &mut Canvas, status: &GameStatus, to_act: Superpower) {
    let mut col = 0;
    let mut put = |canvas: &mut Canvas, text: &str, style: Style| {
        canvas.put(0, col, text, style);
        col += text.chars().count();
    };

    put(canvas, &format!("TURN {} · AR {} · ", status.turn, ar_label(status)), Style::default());
    let side_style = match to_act {
        Superpower::Us => Style::color(Color::Us).bold(),
        Superpower::Ussr => Style::color(Color::Ussr).bold(),
    };
    put(canvas, &format!("{to_act} to act"), side_style);
    put(canvas, &format!(" · DEFCON {} · VP {}{}", status.defcon, vp_line(status.vp), space_label(status)), Style::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{CardCatalog, CardId};
    use crate::game::VictoryReason;
    use crate::country::Superpower;
    use crate::layout::MapLayout;
    use crate::map::WorldMap;
    use crate::ops::InfluencePlacement;
    use crate::render::ColorMode;

    fn fixtures() -> (WorldMap, MapLayout) {
        let map = WorldMap::standard().unwrap();
        let layout = MapLayout::standard(&map).unwrap();
        (map, layout)
    }

    fn status() -> GameStatus {
        GameStatus { turn: 5, action_round: 3, action_rounds_per_turn: 7, active: Superpower::Ussr, defcon: 3, vp: 4, ..Default::default() }
    }

    /// Fidel (card #8, 2 ops) — any non-scoring, non-China card will do.
    fn fidel(cards: &CardCatalog) -> &Card {
        cards.card(CardId(8))
    }

    #[test]
    fn the_bar_is_always_three_rows() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        assert_eq!(render_status_bar(&layout, &board, &status(), None, None, None, 60).height(), STATUS_BAR_ROWS);
        let placement = InfluencePlacement::new(Superpower::Ussr, 4, &board);
        let op = Operation::Influence(placement);
        assert_eq!(render_status_bar(&layout, &board, &status(), None, Some(&op), None, 60).height(), STATUS_BAR_ROWS);
    }

    #[test]
    fn the_first_row_names_the_turn_the_active_side_and_the_score() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let text = render_status_bar(&layout, &board, &status(), None, None, None, 60).render(ColorMode::Never);
        let first_line = text.lines().next().unwrap();
        assert_eq!(first_line, "TURN 5 · AR 3/7 · USSR to act · DEFCON 3 · VP US +4 · Space 0-0");
    }

    #[test]
    fn the_second_row_prompts_to_play_a_card_when_none_is_in_play() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let text = render_status_bar(&layout, &board, &status(), None, None, None, 60).render(ColorMode::Never);
        assert!(text.contains("no card in play"), "missing play hint:\n{text}");
        assert!(text.contains("space play"), "missing play hint:\n{text}");
        assert!(text.contains("p pass"), "missing play hint:\n{text}");
    }

    #[test]
    fn the_second_row_names_the_card_in_play_with_no_operation_open() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let cards = CardCatalog::standard().unwrap();
        let text = render_status_bar(&layout, &board, &status(), Some(fidel(&cards)), None, None, 60).render(ColorMode::Never);
        assert!(text.contains("Fidel"), "missing the card's name:\n{text}");
        assert!(text.contains("2 ops"), "missing the card's ops:\n{text}");
        assert!(text.contains("i influence"), "missing the operation keys:\n{text}");
        assert!(text.contains("return card"), "missing the return-card hint:\n{text}");
        assert!(!text.contains("no card in play"), "shouldn't still prompt to play a card:\n{text}");
    }

    #[test]
    fn a_scoring_card_in_play_shows_the_event_key_instead_of_the_ops_keys() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let cards = CardCatalog::standard().unwrap();
        let europe_scoring = cards.id_by_name("Europe Scoring").unwrap();
        let text = render_status_bar(&layout, &board, &status(), Some(cards.card(europe_scoring)), None, None, 60).render(ColorMode::Never);
        assert!(text.contains("Europe Scoring"), "missing the card's name:\n{text}");
        assert!(text.contains("e score"), "missing the event key:\n{text}");
        assert!(!text.contains("i influence"), "a scoring card has no ops, so the ops keys shouldn't show:\n{text}");
    }

    #[test]
    fn a_winner_replaces_the_card_and_operation_row() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let victory = Victory { side: Superpower::Ussr, reason: VictoryReason::EuropeControl };
        let text = render_status_bar(&layout, &board, &status(), None, None, Some(victory), 60).render(ColorMode::Never);
        assert!(text.contains("GAME OVER"), "missing the game-over row:\n{text}");
        assert!(text.contains("USSR wins (Europe control)"), "missing the winner and reason:\n{text}");
        assert!(!text.contains("no card in play"), "the winner row should replace the card/operation row entirely:\n{text}");
    }

    #[test]
    fn the_second_row_shows_the_open_operations_balance_prefixed_with_the_card() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let cards = CardCatalog::standard().unwrap();
        let placement = InfluencePlacement::new(Superpower::Ussr, 2, &board);
        let op = Operation::Influence(placement);
        let text = render_status_bar(&layout, &board, &status(), Some(fidel(&cards)), Some(&op), None, 60).render(ColorMode::Never);
        assert!(text.contains(&operation_balance_line(&layout, &board, &op)), "balance line missing:\n{text}");
        assert!(text.contains("Fidel"), "should name the card funding the operation:\n{text}");
        assert!(!text.contains("no card in play"), "an open operation shouldn't still prompt to play a card:\n{text}");
    }

    #[test]
    fn the_active_side_is_coloured() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let mut ussr_active = status();
        ussr_active.active = Superpower::Ussr;
        let ussr_text = render_status_bar(&layout, &board, &ussr_active, None, None, None, 60).render(ColorMode::Always);
        assert!(ussr_text.contains('\x1b'), "active side should carry a colour code:\n{ussr_text}");

        let mut us_active = status();
        us_active.active = Superpower::Us;
        let us_text = render_status_bar(&layout, &board, &us_active, None, None, None, 60).render(ColorMode::Always);
        assert_ne!(ussr_text, us_text, "USSR and USA active should render differently under colour");
    }

    #[test]
    fn color_never_emits_no_escape_codes() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let placement = InfluencePlacement::new(Superpower::Ussr, 4, &board);
        let op = Operation::Influence(placement);
        let text = render_status_bar(&layout, &board, &status(), None, Some(&op), None, 60).render(ColorMode::Never);
        assert!(!text.contains('\x1b'), "ColorMode::Never should emit no escapes:\n{text}");
    }

    #[test]
    fn the_bar_is_never_narrower_than_the_requested_width() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        assert_eq!(render_status_bar(&layout, &board, &status(), None, None, None, 200).width(), 200);
    }

    #[test]
    fn no_status_bar_line_exceeds_its_own_width() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let canvas = render_status_bar(&layout, &board, &status(), None, None, None, 10);
        let width = canvas.width();
        for line in canvas.render(ColorMode::Never).lines() {
            assert!(line.chars().count() <= width, "line {line:?} exceeds width {width}");
        }
    }

    #[test]
    fn an_open_event_names_its_chooser_as_the_side_to_act_even_when_the_other_side_is_phasing() {
        let (map, layout) = fixtures();
        let cards = CardCatalog::standard().unwrap();
        let board = Board::new(&map);
        let status = GameStatus { active: Superpower::Us, ..status() };
        // The US is phasing, but Comecon is the USSR's card.
        let comecon = cards.card(CardId(14));
        let choice = crate::events::EventChoice::new(&map, &board, &status, comecon.id).unwrap();
        let op = Operation::Event(Box::new(choice));
        let text = render_status_bar(&layout, &board, &status, Some(comecon), Some(&op), None, 80).render(ColorMode::Never);
        let first = text.lines().next().unwrap();
        assert!(first.contains("USSR to act"), "{first}");
        assert!(!first.contains("USA to act"), "{first}");
        assert!(text.contains("USSR chooses"), "{text}");
        assert!(text.contains("+ add · - remove"), "{text}");
    }

    // --- turn-long effects ---------------------------------------------------

    use crate::ongoing::OngoingEffect;

    fn with_effects(effects: &[OngoingEffect]) -> GameStatus {
        let mut s = status();
        for &e in effects {
            s.effects.apply(e);
        }
        s
    }

    #[test]
    fn the_effects_row_is_a_muted_note_when_nothing_is_in_force_and_never_changes_the_height() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let none = render_status_bar(&layout, &board, &status(), None, None, None, 60);
        let some = render_status_bar(&layout, &board, &with_effects(&[OngoingEffect::Containment]), None, None, None, 60);
        assert_eq!(none.height(), some.height());
        assert!(none.render(ColorMode::Never).lines().nth(2).unwrap().contains("no turn effects"));
    }

    #[test]
    fn the_effects_row_lists_every_effect_in_force_in_card_order() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let s = with_effects(&[OngoingEffect::Chernobyl { region: crate::country::Region::Asia }, OngoingEffect::Containment]);
        let row = render_status_bar(&layout, &board, &s, None, None, None, 60).render(ColorMode::Never).lines().nth(2).unwrap().to_string();
        assert!(row.starts_with("In effect: Containment"), "{row}");
        assert!(row.contains("Chernobyl: USSR can't add influence in Asia"), "{row}");
        assert!(row.find("Containment").unwrap() < row.find("Chernobyl").unwrap(), "{row}");
    }

    #[test]
    fn a_card_in_play_shows_its_ops_as_modified_with_where_the_change_came_from() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let cards = CardCatalog::standard().unwrap();
        // The USSR is active in `status()`; Fidel is a 2-op card.
        let s = with_effects(&[OngoingEffect::Brezhnev]);
        let text = render_status_bar(&layout, &board, &s, Some(fidel(&cards)), None, None, 60).render(ColorMode::Never);
        assert!(text.contains("3 ops: 2 +1 Brezhnev"), "{text}");
        let s = with_effects(&[OngoingEffect::VietnamRevolts]);
        let text = render_status_bar(&layout, &board, &s, Some(fidel(&cards)), None, None, 60).render(ColorMode::Never);
        assert!(text.contains("+1 if all in Southeast Asia"), "{text}");
    }

    #[test]
    fn the_extra_north_sea_oil_round_reads_as_eight_of_seven_plus_one() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let s = GameStatus { action_round: 8, active: Superpower::Us, ..with_effects(&[OngoingEffect::NorthSeaOil]) };
        let first = render_status_bar(&layout, &board, &s, None, None, None, 60).render(ColorMode::Never).lines().next().unwrap().to_string();
        assert!(first.contains("AR 8/7+1"), "{first}");
    }
}
