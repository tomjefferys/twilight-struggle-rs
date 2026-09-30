use crate::board::Board;
use crate::country::Superpower;
use crate::layout::MapLayout;
use crate::map::WorldMap;
use crate::status::GameStatus;

use super::{control_glyph, nz, region_tally, vp_line, Canvas, Color, Style};

/// Width of one country token: 1 battleground flag + 9-char name + 2-digit
/// US influence + 1 control glyph + 2-digit USSR influence.
const TOKEN_WIDTH: usize = 15;
const TOKEN_NAME_WIDTH: usize = 9;
/// Panels are drawn two abreast once the terminal is wide enough for three
/// tokens per panel at that width; below it, each region gets a full-width
/// band instead.
const TWO_PANEL_THRESHOLD: usize = 104;

/// The world dashboard: a status header, then every region as a panel of
/// country tokens, laid out two abreast when `width` allows it and as
/// single full-width bands otherwise.
pub fn render_world(
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    status: &GameStatus,
    width: usize,
) -> Canvas {
    let two_panel = width >= TWO_PANEL_THRESHOLD;
    let panel_width = if two_panel { (width - 2) / 2 } else { width };
    let tokens_per_row = tokens_per_panel(panel_width);
    let drawn_panel_width = 16 * tokens_per_row + 3;

    let regions = layout.region_order();
    let bands: Vec<&[crate::country::Region]> = if two_panel {
        regions.chunks(2).collect()
    } else {
        regions.chunks(1).collect()
    };

    // Never shrink the canvas below the requested width just because the
    // panels themselves are narrower — the header line uses the full
    // width, and panels are left-aligned within it.
    let total_width = if two_panel {
        width.max(drawn_panel_width * 2 + 2)
    } else {
        width.max(drawn_panel_width)
    };

    let panel_heights: Vec<Vec<usize>> = bands
        .iter()
        .map(|band| {
            band.iter()
                .map(|&region| {
                    let count = layout.countries_in_region(map, region).len();
                    let rows = count.div_ceil(tokens_per_row);
                    rows + 2
                })
                .collect()
        })
        .collect();

    let band_heights: Vec<usize> = panel_heights
        .iter()
        .map(|hs| hs.iter().copied().max().unwrap_or(2))
        .collect();

    let header_height = 2;
    let footer_height = 2;
    let total_height =
        header_height + band_heights.iter().sum::<usize>() + footer_height;

    let mut canvas = Canvas::new(total_width.max(1), total_height);

    canvas.put(0, 0, &header_line(status), Style::default());

    let mut row = header_height;
    for (band, band_height) in bands.iter().zip(&band_heights) {
        let mut col = 0;
        for &region in band.iter() {
            draw_region_panel(
                &mut canvas,
                row,
                col,
                drawn_panel_width,
                *band_height,
                map,
                layout,
                board,
                region,
                tokens_per_row,
            );
            col += drawn_panel_width + 2;
        }
        row += band_height;
    }

    row += 1;
    canvas.put(
        row,
        0,
        "  * battleground   4<0 US controls   0>3 USSR controls   1:1 contested",
        Style::color(Color::Muted),
    );

    canvas
}

fn tokens_per_panel(panel_width: usize) -> usize {
    let inner = panel_width.saturating_sub(4);
    ((inner + 1) / 16).max(2)
}

fn header_line(status: &GameStatus) -> String {
    let vp = vp_line(status.vp);
    format!(
        "  TURN {}   AR {}/{} ({})   DEFCON {}   VP {}   Space US {} USSR {}   MilOps US {} USSR {}   China Card: {} ({})",
        status.turn,
        status.action_round,
        status.action_rounds_per_turn,
        status.active,
        status.defcon,
        vp,
        status.space_race_us,
        status.space_race_ussr,
        status.military_ops_us,
        status.military_ops_ussr,
        status.china_card,
        if status.china_card_face_up { "face up" } else { "face down" },
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_region_panel(
    canvas: &mut Canvas,
    row: usize,
    col: usize,
    width: usize,
    height: usize,
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    region: crate::country::Region,
    tokens_per_row: usize,
) {
    let ids = layout.countries_in_region(map, region);
    let tally = region_tally(map, layout, board, region);

    let title = format!(
        " {}  bg {}-{}  ctry {}-{} ",
        region, tally.bg_us, tally.bg_ussr, tally.ctry_us, tally.ctry_ussr
    );
    canvas.draw_box(row, col, width, height, Style::default());
    canvas.put(row, col + 1, &title, Style::default());

    for (i, &id) in ids.iter().enumerate() {
        let token_row = row + 1 + i / tokens_per_row;
        let token_col = col + 2 + (i % tokens_per_row) * (TOKEN_WIDTH + 1);
        draw_token(canvas, token_row, token_col, map, board, id);
    }
}

fn draw_token(
    canvas: &mut Canvas,
    row: usize,
    col: usize,
    map: &WorldMap,
    board: &Board,
    id: crate::country::CountryId,
) {
    let country = map.country(id);
    let us = board.influence(id, Superpower::Us);
    let ussr = board.influence(id, Superpower::Ussr);
    let controller = board.controller(map, id);
    let sep = control_glyph(controller);

    let flag_style = if country.battleground {
        Style::color(Color::Battleground)
    } else {
        Style::default()
    };
    canvas.put_char(
        row,
        col,
        if country.battleground { '*' } else { ' ' },
        flag_style,
    );

    let name: String = country.name.chars().take(TOKEN_NAME_WIDTH).collect();
    canvas.put(row, col + 1, &format!("{:<width$}", name, width = TOKEN_NAME_WIDTH), Style::default());

    let sep_style = match controller {
        Some(Superpower::Us) => Style::color(Color::Us),
        Some(Superpower::Ussr) => Style::color(Color::Ussr),
        None => Style::default(),
    };
    let us_style = if us > 0 { Style::color(Color::Us) } else { Style::color(Color::Muted) };
    let ussr_style = if ussr > 0 { Style::color(Color::Ussr) } else { Style::color(Color::Muted) };

    let stats_col = col + 1 + TOKEN_NAME_WIDTH;
    canvas.put(row, stats_col, &format!("{:>2}", nz(us)), us_style);
    canvas.put_char(row, stats_col + 2, sep, sep_style);
    canvas.put(row, stats_col + 3, &format!("{:<2}", nz(ussr)), ussr_style);
}
