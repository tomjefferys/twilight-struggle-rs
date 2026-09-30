//! The two-row turn/operation bar `interactive.rs` draws above every map
//! screen (world, region, country) — the one thing that makes whose turn
//! it is, and what's currently in progress, unmistakable from a keypress
//! alone. A pure [`Canvas`] producer like every other view; nothing here
//! touches a terminal.

use crate::board::Board;
use crate::country::Superpower;
use crate::layout::MapLayout;
use crate::ops::Operation;
use crate::status::GameStatus;

use super::{operation_balance_line, vp_line, BEGIN_HINT, Canvas, Color, Style};

/// The bar's height, always — with or without an operation open — so the
/// view drawn below it never shifts up or down as one opens or closes.
pub const STATUS_BAR_ROWS: usize = 3;

/// Row 0: turn/AR/active side/DEFCON/VP. Row 1: the open operation's
/// balance (the exact text `operation_balance_line` gives the region and
/// world-map footers, so the three can't disagree), or a prompt naming the
/// keys that start one. Row 2: a plain rule, separating the bar from
/// whichever screen it sits above.
///
/// `board` should be the caller's *committed* board, not a speculative
/// one — `operation_balance_line` only reads it through a realignment's or
/// coup's `delta`, which always tracks the real board regardless (a
/// placement's own pending points come from `op` itself).
///
/// `width` is a minimum, not a fixed size: the canvas grows to fit
/// whichever row is longer, the same rule `render_world`/`render_region`
/// already follow for their own content, so nothing here is ever clipped.
pub fn render_status_bar(layout: &MapLayout, board: &Board, status: &GameStatus, op: Option<&Operation>, width: usize) -> Canvas {
    let turn_line = format!(
        "TURN {} · AR {}/{} · {} to act · DEFCON {} · VP {}",
        status.turn,
        status.action_round,
        status.action_rounds_per_turn,
        status.active,
        status.defcon,
        vp_line(status.vp),
    );
    let (op_line, op_style) = match op {
        Some(operation) => (operation_balance_line(layout, board, operation), Style::color(Color::Selected)),
        None => (format!("no operation open — {BEGIN_HINT}"), Style::color(Color::Muted)),
    };

    let content_width = width.max(turn_line.chars().count()).max(op_line.chars().count()).max(1);
    let mut canvas = Canvas::new(content_width, STATUS_BAR_ROWS);

    draw_turn_line(&mut canvas, status);
    canvas.put(1, 0, &op_line, op_style);
    canvas.put(2, 0, &"─".repeat(content_width), Style::color(Color::Muted));

    canvas
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

    #[test]
    fn the_bar_is_always_three_rows() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        assert_eq!(render_status_bar(&layout, &board, &status(), None, 60).height(), STATUS_BAR_ROWS);
        let placement = InfluencePlacement::new(Superpower::Ussr, 4, &board);
        let op = Operation::Influence(placement);
        assert_eq!(render_status_bar(&layout, &board, &status(), Some(&op), 60).height(), STATUS_BAR_ROWS);
    }

    #[test]
    fn the_first_row_names_the_turn_the_active_side_and_the_score() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let text = render_status_bar(&layout, &board, &status(), None, 60).render(ColorMode::Never);
        let first_line = text.lines().next().unwrap();
        assert_eq!(first_line, "TURN 5 · AR 3/7 · USSR to act · DEFCON 3 · VP US +4");
    }

    #[test]
    fn the_second_row_prompts_for_an_operation_when_none_is_open() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let text = render_status_bar(&layout, &board, &status(), None, 60).render(ColorMode::Never);
        assert!(text.contains("i influence"), "missing begin hint:\n{text}");
        assert!(text.contains("a realign"), "missing begin hint:\n{text}");
        assert!(text.contains("o coup"), "missing begin hint:\n{text}");
        assert!(text.contains("p pass"), "missing begin hint:\n{text}");
    }

    #[test]
    fn the_second_row_shows_the_open_operations_balance() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let placement = InfluencePlacement::new(Superpower::Ussr, 4, &board);
        let op = Operation::Influence(placement);
        let text = render_status_bar(&layout, &board, &status(), Some(&op), 60).render(ColorMode::Never);
        assert!(text.contains(&operation_balance_line(&layout, &board, &op)), "balance line missing:\n{text}");
        assert!(!text.contains("i influence"), "an open operation shouldn't still prompt to start one:\n{text}");
    }

    #[test]
    fn the_active_side_is_coloured() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let mut ussr_active = status();
        ussr_active.active = Superpower::Ussr;
        let ussr_text = render_status_bar(&layout, &board, &ussr_active, None, 60).render(ColorMode::Always);
        assert!(ussr_text.contains('\x1b'), "active side should carry a colour code:\n{ussr_text}");

        let mut us_active = status();
        us_active.active = Superpower::Us;
        let us_text = render_status_bar(&layout, &board, &us_active, None, 60).render(ColorMode::Always);
        assert_ne!(ussr_text, us_text, "USSR and USA active should render differently under colour");
    }

    #[test]
    fn color_never_emits_no_escape_codes() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let placement = InfluencePlacement::new(Superpower::Ussr, 4, &board);
        let op = Operation::Influence(placement);
        let text = render_status_bar(&layout, &board, &status(), Some(&op), 60).render(ColorMode::Never);
        assert!(!text.contains('\x1b'), "ColorMode::Never should emit no escapes:\n{text}");
    }

    #[test]
    fn the_bar_is_never_narrower_than_the_requested_width() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        assert_eq!(render_status_bar(&layout, &board, &status(), None, 200).width(), 200);
    }

    #[test]
    fn no_status_bar_line_exceeds_its_own_width() {
        let (map, layout) = fixtures();
        let board = Board::new(&map);
        let canvas = render_status_bar(&layout, &board, &status(), None, 10);
        let width = canvas.width();
        for line in canvas.render(ColorMode::Never).lines() {
            assert!(line.chars().count() <= width, "line {line:?} exceeds width {width}");
        }
    }
}
