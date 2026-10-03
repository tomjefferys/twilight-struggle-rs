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
use crate::status::GameStatus;

use super::{game_over_line, operation_balance_line, vp_line, Canvas, Color, Style};

/// Row 1's wording when no card is in play yet — the status bar's own
/// three-state hint (see [`render_status_bar`]'s own doc), distinct from
/// `BEGIN_HINT`'s card-agnostic one-liner shown by the region/world-map/
/// country views, which never know whether a card's already been played.
const PLAY_HINT: &str = "no card in play — [ ] select · space play · p pass";

/// The bar's height, always — with or without an operation open — so the
/// view drawn below it never shifts up or down as one opens or closes.
pub const STATUS_BAR_ROWS: usize = 3;

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
/// Row 2: a plain rule, separating the bar from whichever screen it sits
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
    let turn_line = format!(
        "TURN {} · AR {}/{} · {} to act · DEFCON {} · VP {}",
        status.turn,
        status.action_round,
        status.action_rounds_per_turn,
        status.active,
        status.defcon,
        vp_line(status.vp),
    );
    let (op_line, op_style) = if let Some(victory) = winner {
        (game_over_line(victory), side_style(victory.side).bold())
    } else {
        match (card, op) {
            (_, Some(operation)) => {
                let name = card.map(|c| c.name.as_str()).unwrap_or("?");
                (format!("{name} · {}", operation_balance_line(layout, board, operation)), Style::color(Color::Selected))
            }
            (Some(card), None) if card.scoring => {
                (format!("playing {} — e score · ⌫ return card", card.name), Style::color(Color::Selected))
            }
            (Some(card), None) if events::is_implemented(card.id) => (
                format!("playing {} ({} ops) — e event · i influence · a realign · o coup · ⌫ return card", card.name, card.ops),
                Style::color(Color::Selected),
            ),
            (Some(card), None) => (
                format!("playing {} ({} ops) — i influence · a realign · o coup · ⌫ return card", card.name, card.ops),
                Style::color(Color::Selected),
            ),
            (None, None) => (PLAY_HINT.to_string(), Style::color(Color::Muted)),
        }
    };

    let content_width = width.max(turn_line.chars().count()).max(op_line.chars().count()).max(1);
    let mut canvas = Canvas::new(content_width, STATUS_BAR_ROWS);

    draw_turn_line(&mut canvas, status);
    canvas.put(1, 0, &op_line, op_style);
    canvas.put(2, 0, &"─".repeat(content_width), Style::color(Color::Muted));

    canvas
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
fn draw_turn_line(canvas: &mut Canvas, status: &GameStatus) {
    let mut col = 0;
    let mut put = |canvas: &mut Canvas, text: &str, style: Style| {
        canvas.put(0, col, text, style);
        col += text.chars().count();
    };

    put(canvas, &format!("TURN {} · AR {}/{} · ", status.turn, status.action_round, status.action_rounds_per_turn), Style::default());
    let side_style = match status.active {
        Superpower::Us => Style::color(Color::Us).bold(),
        Superpower::Ussr => Style::color(Color::Ussr).bold(),
    };
    put(canvas, &format!("{} to act", status.active), side_style);
    put(canvas, &format!(" · DEFCON {} · VP {}", status.defcon, vp_line(status.vp)), Style::default());
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
        assert_eq!(first_line, "TURN 5 · AR 3/7 · USSR to act · DEFCON 3 · VP US +4");
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
}
