//! Turning game state into text.
//!
//! Every view here is a pure function from `(WorldMap, MapLayout, Board,
//! ...)` to a [`Canvas`], and a `Canvas` only ever turns into a `String` —
//! nothing in this module touches the terminal directly. That keeps every
//! view snapshot-testable, and leaves the choice of how it's actually
//! presented (a REPL printing to stdout today, perhaps a full-screen TUI
//! later) entirely outside this module.

pub mod card;
mod chip;
pub mod country;
pub mod event;
pub mod hand;
pub mod log;
pub mod region;
pub mod roll;
pub mod score;
pub mod space;
pub mod statusbar;
pub mod trap;
pub mod war;
pub mod world;
pub mod worldmap;

pub use card::render_card;
pub use country::render_country;
pub use event::{render_event_result, render_event_session};
pub use hand::{render_hand, HAND_ROWS, HAND_WIDTH};
pub use log::{log_entry_line, log_text, render_log};
pub use region::render_region;
pub use roll::{render_roll_result, RollReport};
pub use score::render_scoring_result;
pub use trap::{render_trap_confirm, render_trap_result};
pub use space::{render_space_confirm, render_space_result, render_space_track, render_space_track_with_hint};
pub use statusbar::{render_status_bar, render_status_bar_with, STATUS_BAR_ROWS};
pub use war::render_war_result;
pub use world::render_world;
pub use worldmap::render_world_map;

/// A semantic colour. What each one actually looks like is up to the
/// [`Theme`] resolving it, so a colour-blind or light-background palette is
/// a data change here, not a rewrite of every view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Us,
    Ussr,
    Battleground,
    Muted,
    /// The six *Twilight Struggle* board regions, each matching that
    /// region's colour on the physical board — used to tint the world
    /// map's landmass so it reads like the real board at a glance.
    Europe,
    Asia,
    MiddleEast,
    Africa,
    CentralAmerica,
    SouthAmerica,
    /// A UI accent with no in-game meaning of its own — used to mark
    /// whatever is currently selected in interactive navigation, so it
    /// doesn't compete with (or get lost among) the colours above, which
    /// all mean something about the game state.
    Selected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub color: Color,
    pub bold: bool,
    pub dim: bool,
}

impl Style {
    pub fn color(color: Color) -> Style {
        Style { color, bold: false, dim: false }
    }

    pub fn bold(mut self) -> Style {
        self.bold = true;
        self
    }

    /// A subdued version of the same colour — used for background
    /// shading, so it recedes behind the foreground content drawn on top.
    pub fn dim(mut self) -> Style {
        self.dim = true;
        self
    }
}

/// Whether [`Canvas::render`] emits ANSI colour codes at all. `Never`
/// guarantees the output contains no escape sequences, for piped output,
/// `NO_COLOR`, and snapshot tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Always,
    Never,
}

/// Whether a view is being drawn once into scrollback (`Static`, the REPL's
/// one-shot commands) or redrawn every keypress by [`crate::interactive`]
/// (`Interactive`). The only view that reads this today is
/// [`country::render_country`], which draws a key-hint row under
/// `Interactive` and omits it under `Static` — there's nothing to hint at
/// in a view that isn't listening for keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Static,
    Interactive,
}

/// Resolves a [`Style`] to an ANSI SGR sequence.
struct Theme;

impl Theme {
    fn sgr(style: Style) -> Option<String> {
        // The base 16-colour palette can't cover orange/purple, so those
        // two reach for a 256-colour code; everything else stays a plain
        // SGR number for maximum terminal compatibility.
        let color_code: Option<&'static str> = match style.color {
            Color::Default => None,
            Color::Us => Some("94"),
            Color::Ussr => Some("91"),
            Color::Battleground => Some("93"),
            Color::Muted => None,
            Color::Europe => Some("38;5;140"),
            Color::Asia => Some("38;5;208"),
            Color::MiddleEast => Some("96"),
            Color::Africa => Some("33"),
            // Bright green and plain green (the original pair) read as
            // near-identical at a glance, especially tinting a whole
            // region's worth of boxes at once — Central America moves to
            // magenta, still a base-16 code like every other colour here
            // except the two that need a 256-colour one (see above).
            Color::CentralAmerica => Some("95"),
            Color::SouthAmerica => Some("32"),
            Color::Selected => Some("97"),
        };

        let mut parts: Vec<&str> = Vec::new();
        if style.bold {
            parts.push("1");
        }
        if style.dim || style.color == Color::Muted {
            parts.push("2");
        }
        if let Some(code) = color_code {
            parts.push(code);
        }

        if parts.is_empty() {
            None
        } else {
            Some(format!("\x1b[{}m", parts.join(";")))
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CanvasCell {
    ch: char,
    style: Style,
}

impl Default for CanvasCell {
    fn default() -> Self {
        CanvasCell {
            ch: ' ',
            style: Style::default(),
        }
    }
}

/// A fixed-size character grid that views draw into. Out-of-bounds writes
/// are silently clipped rather than panicking, since a view's content can
/// legitimately vary in size (a wide token at high influence, say) without
/// that being worth treating as a bug.
pub struct Canvas {
    width: usize,
    rows: Vec<Vec<CanvasCell>>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Canvas {
            width,
            rows: vec![vec![CanvasCell::default(); width]; height],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.rows.len()
    }

    pub fn put_char(&mut self, row: usize, col: usize, ch: char, style: Style) {
        if let Some(cell) = self.rows.get_mut(row).and_then(|r| r.get_mut(col)) {
            *cell = CanvasCell { ch, style };
        }
    }

    pub fn put(&mut self, row: usize, col: usize, text: &str, style: Style) {
        for (i, ch) in text.chars().enumerate() {
            self.put_char(row, col + i, ch, style);
        }
    }

    pub fn draw_box(&mut self, row: usize, col: usize, w: usize, h: usize, style: Style) {
        self.draw_box_glyphs(row, col, w, h, style, ['┌', '┐', '└', '┘', '─', '│']);
    }

    /// The same box as [`Canvas::draw_box`], but drawn with heavy
    /// box-drawing characters instead of thin ones — a shape change, not
    /// just a style change, so a selection drawn this way still stands
    /// out under [`ColorMode::Never`], where colour and boldness are both
    /// invisible.
    pub fn draw_thick_box(&mut self, row: usize, col: usize, w: usize, h: usize, style: Style) {
        self.draw_box_glyphs(row, col, w, h, style, ['┏', '┓', '┗', '┛', '━', '┃']);
    }

    /// The same box again with double-line characters — what a chip an
    /// open event can act on is drawn with, so "eligible" is a shape too,
    /// not just a colour (visible under [`ColorMode::Never`]).
    pub fn draw_double_box(&mut self, row: usize, col: usize, w: usize, h: usize, style: Style) {
        self.draw_box_glyphs(row, col, w, h, style, ['╔', '╗', '╚', '╝', '═', '║']);
    }

    /// A `├── Title ───┤` row inside an existing box: the section
    /// separator the country view's panels are built from. `title` is
    /// written two columns in, with a space either side; passing `""`
    /// draws a plain rule with no label.
    pub fn draw_divider(&mut self, row: usize, col: usize, w: usize, title: &str, style: Style) {
        if w < 2 {
            return;
        }
        self.put_char(row, col, '├', style);
        self.put_char(row, col + w - 1, '┤', style);
        for c in (col + 1)..(col + w - 1) {
            self.put_char(row, c, '─', style);
        }
        if !title.is_empty() {
            self.put(row, col + 2, &format!(" {title} "), style);
        }
    }

    fn draw_box_glyphs(&mut self, row: usize, col: usize, w: usize, h: usize, style: Style, glyphs: [char; 6]) {
        if w < 2 || h < 2 {
            return;
        }
        let [top_left, top_right, bottom_left, bottom_right, horizontal, vertical] = glyphs;
        self.put_char(row, col, top_left, style);
        self.put_char(row, col + w - 1, top_right, style);
        self.put_char(row + h - 1, col, bottom_left, style);
        self.put_char(row + h - 1, col + w - 1, bottom_right, style);
        for c in (col + 1)..(col + w - 1) {
            self.put_char(row, c, horizontal, style);
            self.put_char(row + h - 1, c, horizontal, style);
        }
        for r in (row + 1)..(row + h - 1) {
            self.put_char(r, col, vertical, style);
            self.put_char(r, col + w - 1, vertical, style);
        }
    }

    /// Opaquely copies every cell of `src` onto `self`, anchored at
    /// `(row, col)` — including `src`'s own blank cells, so this paints
    /// over whatever was already there rather than punching a
    /// transparent hole the shape of `src`. Used to draw a card's zoom
    /// overlay on top of a map screen. Silently clipped at either edge,
    /// like every other `Canvas` write.
    pub fn blit(&mut self, src: &Canvas, row: usize, col: usize) {
        for (r, line) in src.rows.iter().enumerate() {
            for (c, cell) in line.iter().enumerate() {
                self.put_char(row + r, col + c, cell.ch, cell.style);
            }
        }
    }

    /// Renders the canvas as text, one line per row, with trailing spaces
    /// on each line trimmed. `mode` controls whether ANSI colour codes are
    /// emitted at all — `Never` guarantees no `\x1b` appears anywhere in
    /// the output.
    pub fn render(&self, mode: ColorMode) -> String {
        self.rows
            .iter()
            .map(|row| render_line(row, mode))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn render_line(cells: &[CanvasCell], mode: ColorMode) -> String {
    let mut end = cells.len();
    while end > 0 && cells[end - 1].ch == ' ' {
        end -= 1;
    }
    let cells = &cells[..end];

    if mode == ColorMode::Never {
        return cells.iter().map(|c| c.ch).collect();
    }

    let mut out = String::new();
    let mut i = 0;
    while i < cells.len() {
        let style = cells[i].style;
        let mut j = i + 1;
        while j < cells.len() && cells[j].style == style {
            j += 1;
        }
        let text: String = cells[i..j].iter().map(|c| c.ch).collect();
        match Theme::sgr(style) {
            Some(code) => {
                out.push_str(&code);
                out.push_str(&text);
                out.push_str("\x1b[0m");
            }
            None => out.push_str(&text),
        }
        i = j;
    }
    out
}

/// Battleground and total country counts for one region, US then USSR —
/// the numbers shown in a region's title, whether that's a dashboard panel
/// or the world map's selection line.
pub(crate) struct RegionTally {
    pub bg_us: usize,
    pub bg_ussr: usize,
    pub ctry_us: usize,
    pub ctry_ussr: usize,
}

pub(crate) fn region_tally(
    map: &crate::map::WorldMap,
    layout: &crate::layout::MapLayout,
    board: &crate::board::Board,
    region: crate::country::Region,
) -> RegionTally {
    use crate::country::Superpower;

    let ids = layout.countries_in_region(map, region);
    let bg_us = ids
        .iter()
        .filter(|&&id| map.country(id).battleground && board.is_controlled_by(map, id, Superpower::Us))
        .count();
    let bg_ussr = ids
        .iter()
        .filter(|&&id| map.country(id).battleground && board.is_controlled_by(map, id, Superpower::Ussr))
        .count();
    let ctry_us = ids.iter().filter(|&&id| board.is_controlled_by(map, id, Superpower::Us)).count();
    let ctry_ussr = ids.iter().filter(|&&id| board.is_controlled_by(map, id, Superpower::Ussr)).count();

    RegionTally { bg_us, bg_ussr, ctry_us, ctry_ussr }
}

/// The board colour for one of the six *Twilight Struggle* regions —
/// matches that region's colour on the physical board, independent of any
/// country's control state. Shared by the world map's landmass tint
/// (`worldmap::zone_color`, which also covers the two superpower colours)
/// and the region view's guest chips (`region::draw_guest_country_box`),
/// so there's one `Region → Color` table rather than two drifting apart.
pub(crate) fn region_color(region: crate::country::Region) -> Color {
    use crate::country::Region;
    match region {
        Region::Europe => Color::Europe,
        Region::Asia => Color::Asia,
        Region::MiddleEast => Color::MiddleEast,
        Region::Africa => Color::Africa,
        Region::CentralAmerica => Color::CentralAmerica,
        Region::SouthAmerica => Color::SouthAmerica,
    }
}

/// The colour a card's own side tints it — the hand strip's ops badge and
/// the zoom view's border. [`crate::cards::CardSide::Neutral`] gets
/// [`Color::Default`] rather than a colour of its own: a scoring card or a
/// both-sides event isn't "the third side," it's simply uncoloured, the
/// same way a tied country's control glyph is `:` rather than a third
/// colour.
pub(crate) fn card_side_color(side: crate::cards::CardSide) -> Color {
    use crate::cards::CardSide;
    match side {
        CardSide::Us => Color::Us,
        CardSide::Ussr => Color::Ussr,
        CardSide::Neutral => Color::Default,
    }
}

/// Greedy word-wrap: breaks `text` (already a single paragraph, with
/// ordinary spaces — no embedded `\n`s in any card's own rules text) into
/// lines of at most `width` characters, breaking only at spaces. A single
/// word longer than `width` is left on its own line rather than split
/// mid-word, so the caller's own width is a target, not a hard cap, in
/// that one edge case.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate_len = if current.is_empty() { word.chars().count() } else { current.chars().count() + 1 + word.chars().count() };
        if !current.is_empty() && candidate_len > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// `"-"` for zero, otherwise the number — so an occupied country stands out
/// from an empty one at a glance.
pub(crate) fn nz(value: u8) -> String {
    if value == 0 {
        "-".to_string()
    } else {
        value.to_string()
    }
}

/// Writes a box's top-border title pair: `left` two columns in with a
/// space either side, `right` the same way flush to the border's right
/// edge — the `┌─ * Poland ─────────── Europe · stability 3 ─┐` pattern
/// the country view's outer box uses, matching the space padding
/// [`Canvas::draw_divider`] already gives its own titles. Each half keeps
/// its own style (the name is bolded, or battleground-coloured; the
/// region/stability half stays plain) — both are written directly over
/// the border characters `draw_box`/`draw_thick_box` already drew there,
/// leaving the dash immediately after the corner untouched on each side.
#[allow(clippy::too_many_arguments)]
pub(crate) fn put_border_title(canvas: &mut Canvas, row: usize, col: usize, left: &str, left_style: Style, right: &str, right_style: Style, w: usize) {
    canvas.put(row, col + 2, &format!(" {left} "), left_style);
    if !right.is_empty() {
        let text = format!(" {right} ");
        let start = (col + w).saturating_sub(2 + text.chars().count());
        canvas.put(row, start, &text, right_style);
    }
}

/// The control-marker glyph: `<` for US, `>` for USSR, `:` for contested —
/// doubling as the separator between the two influence numbers.
pub(crate) fn control_glyph(controller: Option<crate::country::Superpower>) -> char {
    match controller {
        Some(crate::country::Superpower::Us) => '<',
        Some(crate::country::Superpower::Ussr) => '>',
        None => ':',
    }
}

/// The ops balance line shown wherever an operation is in progress: the
/// region view's footer, the world map's footer, and — since neither the
/// six-region dashboard nor the country detail view have room to show
/// pending state inline — a banner `main.rs` prints above them instead.
/// Worded the same way for every operation (just naming its own verb),
/// so switching views mid-action reads as the same session, not several.
///
/// For a realignment, the number shown per country is primarily the
/// *opponent's* net change there — the operation's own core metric,
/// mirroring how a placement's number is always the placing side's own
/// points added. A country that's also cost the acting side its own
/// influence (a later roll there went the other way) gets that named
/// too; a pure tie reads as "tied" rather than a bare `+0`.
pub fn operation_balance_line(layout: &crate::layout::MapLayout, board: &crate::board::Board, op: &crate::ops::Operation) -> String {
    if let crate::ops::Operation::Event(e) = op {
        return format!("{} · {} · {}", operation_header(op), e.prompt(), operation_touched_line(layout, board, op));
    }
    format!("{} · {}", operation_header(op), operation_touched_line(layout, board, op))
}

/// The key hint under a map screen while an event settled by its mode alone
/// is open (a discard decision, a DEFCON level): browsing the map is still
/// fine, but nothing on it is the event's business.
pub(crate) fn mode_only_hint(e: &crate::events::EventChoice) -> String {
    if e.needs_roll() && e.is_participation() {
        "←→↑↓ look around · r roll the dice · 1-2 change · ⌫ clear".to_string()
    } else if e.needs_roll() {
        "←→↑↓ look around · r roll the dice · ⌫ cancel the event".to_string()
    } else if e.gate_cards().is_empty() {
        format!("←→↑↓ look around · 1-{} choose · ⌫ undo · c done", e.modes().len().min(9))
    } else {
        if e.gate_offset() == 0 {
            "←→↑↓ look around · [ ] pick a card · space discard it · ⌫ undo · c done".to_string()
        } else {
            "←→↑↓ look around · [ ] pick a card · space discard it · 1 keep your cards · ⌫ undo · c done".to_string()
        }
    }
}

/// `GAME OVER — USSR wins (VP)` — [`statusbar::render_status_bar`]'s own
/// winner row, shared so `main.rs`'s `status`/`event` commands print the
/// exact same wording when there's no screen to draw it on.
pub fn game_over_line(victory: crate::game::Victory) -> String {
    let reason = match victory.reason {
        crate::game::VictoryReason::Vp => "VP",
        crate::game::VictoryReason::EuropeControl => "Europe control",
        crate::game::VictoryReason::Defcon => "DEFCON 1",
        crate::game::VictoryReason::Wargames => "Wargames",
        crate::game::VictoryReason::CubanMissileCrisis => "Cuban Missile Crisis",
    };
    format!("GAME OVER — {} wins ({reason})", victory.side)
}

/// The side/verb/ops-remaining half of [`operation_balance_line`] on its
/// own — the country view's Operation panel uses this as its title and
/// draws [`operation_touched_line`] as a row inside instead of gluing the
/// two together on one line the way the region and world-map footers do.
pub fn operation_header(op: &crate::ops::Operation) -> String {
    if let crate::ops::Operation::War(w) = op {
        return format!("{} declaring war · choose a target (needs {}+)", w.side(), w.success_min());
    }
    if let crate::ops::Operation::Event(e) = op {
        return format!("{} chooses · {}", op.side(), e.progress());
    }
    let pending = op.pending_bonus();
    let bonus = if pending.is_empty() {
        String::new()
    } else {
        format!(" ({})", pending.iter().map(|b| format!("+{} if all in {}", b.ops, b.area)).collect::<Vec<_>>().join(", "))
    };
    format!("{} {} · {} of {} ops left{bonus}", op.side(), op.verb(), op.remaining(), op.ops_total())
}

/// One turn-long event, worded for the status bar, the REPL's `status`
/// and the event's own result — `Containment: US ops +1`. The side to
/// colour it by is [`crate::ongoing::OngoingEffect::side`].
pub fn ongoing_effect_line(effect: &crate::ongoing::OngoingEffect) -> String {
    use crate::ongoing::OngoingEffect as E;
    match effect {
        E::VietnamRevolts => "Vietnam Revolts: USSR ops +1 if all in Southeast Asia".to_string(),
        E::Containment => "Containment: US ops +1".to_string(),
        E::RedScare { penalised } => format!("Red Scare/Purge: {penalised} ops -1"),
        E::NuclearSubs => "Nuclear Subs: US battleground coups keep DEFCON".to_string(),
        E::Brezhnev => "Brezhnev Doctrine: USSR ops +1".to_string(),
        E::DeathSquads { beneficiary } => {
            format!("Death Squads: {beneficiary} coups +1 / {} -1 in Central & South America", beneficiary.opponent())
        }
        E::NorthSeaOil => "North Sea Oil: US plays an 8th action round".to_string(),
        E::IranContra => "Iran-Contra: US realignment rolls -1".to_string(),
        E::Chernobyl { region } => format!("Chernobyl: USSR can't add influence in {region} with ops"),
        E::YuriSamantha => "Yuri and Samantha: USSR +1 VP per US coup".to_string(),
        E::CubanMissileCrisis { by } => format!("Cuban Missile Crisis: a {} coup loses the game (d to defuse)", by.opponent()),
        E::HandRevealed { side, .. } => format!("{side} hand revealed to {} (v to view it)", side.opponent()),
    }
}

/// One game-long event, worded like [`ongoing_effect_line`] — `NATO: US-held Europe
/// is safe from USSR coups and realignment`. Coloured by
/// [`crate::ongoing::LastingEffect::side`].
pub fn lasting_effect_line(effect: &crate::ongoing::LastingEffect) -> String {
    use crate::ongoing::LastingEffect as E;
    match effect {
        E::DeGaulle => "De Gaulle: France is outside NATO".to_string(),
        E::Nato => "NATO: USSR can't coup/realign US-held Europe".to_string(),
        E::UsJapan => "US/Japan Pact: USSR can't coup/realign Japan".to_string(),
        E::Formosan => "Formosan Resolution: US-held Taiwan scores as a battleground".to_string(),
        E::WeWillBuryYou { .. } => "We Will Bury You: USSR +3 VP after the US's next round".to_string(),
        E::WillyBrandt => "Willy Brandt: West Germany is outside NATO".to_string(),
        E::FlowerPower => "Flower Power: USSR +2 VP per US war card".to_string(),
        E::ShuttleDiplomacy => "Shuttle Diplomacy: -1 USSR battleground at next Asia/Middle East scoring".to_string(),
        E::Quagmire => "Quagmire: US action rounds are escape attempts (discard 2+ ops, roll 1-4)".to_string(),
        E::BearTrap => "Bear Trap: USSR action rounds are escape attempts (discard 2+ ops, roll 1-4)".to_string(),
        E::Norad => "NORAD: +1 US influence when a round moves DEFCON to 2 (US holds Canada)".to_string(),
    }
}

/// The "where has this operation acted so far" half of
/// [`operation_balance_line`] on its own. See that function's own doc for
/// what each operation kind's per-country summary says.
pub fn operation_touched_line(layout: &crate::layout::MapLayout, board: &crate::board::Board, op: &crate::ops::Operation) -> String {
    let touched = op.touched();
    if touched.is_empty() {
        "nothing yet".to_string()
    } else {
        touched
            .iter()
            .map(|&id| touched_country_summary(layout, board, op, id))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn touched_country_summary(
    layout: &crate::layout::MapLayout,
    board: &crate::board::Board,
    op: &crate::ops::Operation,
    id: crate::country::CountryId,
) -> String {
    let name = layout.short_name(id);
    match op {
        crate::ops::Operation::Influence(p) => format!("{name} +{}", p.pending(id)),
        crate::ops::Operation::Event(_) => {
            let changes: Vec<String> = [crate::country::Superpower::Us, crate::country::Superpower::Ussr]
                .into_iter()
                .filter_map(|s| match op.delta(board, id, s) {
                    0 => None,
                    d => Some(format!("{s}{d:+}")),
                })
                .collect();
            format!("{name} {}", changes.join("/"))
        }
        crate::ops::Operation::War(_) => name.to_string(),
        crate::ops::Operation::Realign(_) | crate::ops::Operation::Coup(_) => {
            let side = op.side();
            let opponent = side.opponent();
            let opp_delta = op.delta(board, id, opponent);
            let own_delta = op.delta(board, id, side);
            // A pure (0, 0) reads differently depending on how it can
            // happen: a realignment roll actually contested the two
            // totals and came out level, so "tied" fits; a coup with
            // nothing changed only ever means its one attempt fell short
            // of the target number, so "failed" is the honest word.
            let no_change = match op {
                crate::ops::Operation::Coup(_) => format!("{name} failed"),
                _ => format!("{name} tied"),
            };
            match (opp_delta, own_delta) {
                (0, 0) => no_change,
                (o, 0) => format!("{name} {opponent}{o:+}"),
                (0, s) => format!("{name} {side}{s:+}"),
                (o, s) => format!("{name} {opponent}{o:+}/{side}{s:+}"),
            }
        }
    }
}

/// What lowered a war's die roll, in words: `US controls Japan, Taiwan`,
/// or `no US-controlled neighbours`.
pub fn war_reasons(map: &crate::map::WorldMap, m: &crate::events::war::WarModifier) -> String {
    let mut parts: Vec<String> = m.neighbours.iter().map(|&n| map.country(n).name.clone()).collect();
    if m.target_itself {
        parts.insert(0, "the target itself".to_string());
    }
    if parts.is_empty() {
        "no enemy-controlled neighbours".to_string()
    } else {
        format!("enemy controls {}", parts.join(", "))
    }
}

/// A war target's preview, shared by the region footer and the country
/// view: `USSR  d6 -2 · needs 4+ · wins on 1 of 6 (enemy controls Japan, Taiwan)`.
pub fn war_line(map: &crate::map::WorldMap, board: &crate::board::Board, war: &crate::events::War, id: crate::country::CountryId) -> String {
    let m = war.modifier(map, board, id);
    format!(
        "{}  d6 {:+} · needs {}+ · wins on {} of 6 ({})",
        war.side(),
        m.total(),
        war.success_min(),
        war.odds(map, board, id),
        war_reasons(map, &m)
    )
}

/// One side's realignment modifier breakdown for the currently selected
/// country — shared by the region footer, the country detail view, and
/// the REPL's own `target`/`roll` output, so all three read identically.
pub fn modifier_line(side: crate::country::Superpower, mods: &crate::ops::Modifiers) -> String {
    let reasons = mods.reasons();
    let detail = if reasons.is_empty() { "no modifiers".to_string() } else { reasons.join(" · ") };
    format!("{side}  d6 {:+}   ({detail})", mods.total())
}

/// The outcome of one resolved realignment roll, in a single line — used
/// by both the REPL's own `roll` command and the interactive footer's
/// sticky roll message, so the wording is identical either way.
pub fn roll_result_line(map: &crate::map::WorldMap, side: crate::country::Superpower, result: &crate::ops::RollResult) -> String {
    let opponent = side.opponent();
    let country = &map.country(result.target).name;
    let acting_total = result.acting_die as i8 + result.acting_mods.total();
    let opposing_total = result.opposing_die as i8 + result.opposing_mods.total();
    let dice = format!(
        "{side} {}{:+}={acting_total} · {opponent} {}{:+}={opposing_total}",
        result.acting_die, result.acting_mods.total(), result.opposing_die, result.opposing_mods.total(),
    );
    let outcome = match result.loser {
        None => format!("a tie — no influence removed in {country}"),
        Some(loser) if loser == opponent => format!("{side} removes {} {opponent} influence from {country}", result.removed),
        Some(_) => format!("{opponent} wins the roll — {side} loses {} of its own influence in {country}", result.removed),
    };
    format!("{dice} → {outcome}")
}

/// The realignment odds line: each side's win/draw/loss share out of 36,
/// plus the expected influence swing on each side.
pub fn odds_line(side: crate::country::Superpower, odds: &crate::ops::Odds) -> String {
    let opponent = side.opponent();
    format!(
        "odds  {side} {}/36 · draw {}/36 · {opponent} {}/36   avg  {opponent} -{:.1} / {side} -{:.1}",
        odds.win,
        odds.draw,
        odds.loss,
        odds.removed_36ths as f32 / 36.0,
        odds.lost_36ths as f32 / 36.0,
    )
}

/// A coup's target number for the currently selected country, alongside
/// what's rolled against it — the `modifier_line` analogue for a coup,
/// shared by the region footer, the country detail view, and the REPL.
pub fn coup_target_line(side: crate::country::Superpower, ops: u8, target_number: u8, stability: u8) -> String {
    coup_target_line_with(side, ops, 0, target_number, stability)
}

/// [`coup_target_line`] with an ongoing event's die `modifier` shown too.
pub fn coup_target_line_with(side: crate::country::Superpower, ops: u8, modifier: i8, target_number: u8, stability: u8) -> String {
    let modifier = if modifier == 0 { String::new() } else { format!(" {modifier:+}")};
    format!("{side}  d6 +{ops}{modifier} vs {target_number}   (stability {stability} ×2)")
}

/// The coup odds line: success/failure share out of 6, plus the expected
/// influence swing on each side.
pub fn coup_odds_line(side: crate::country::Superpower, odds: &crate::ops::CoupOdds) -> String {
    let opponent = side.opponent();
    format!(
        "odds  {side} {}/6 · {opponent} {}/6   avg  {opponent} -{:.1} / {side} +{:.1}",
        odds.success,
        odds.failure,
        odds.removed_6ths as f32 / 6.0,
        odds.added_6ths as f32 / 6.0,
    )
}

/// The outcome of a resolved coup attempt, in a single line — used by
/// both the REPL's own `roll` command and the interactive footer's
/// sticky roll message, so the wording is identical either way.
pub fn coup_result_line(map: &crate::map::WorldMap, side: crate::country::Superpower, result: &crate::ops::CoupResult) -> String {
    let opponent = side.opponent();
    let country = &map.country(result.target).name;
    let modified = result.die as i16 + result.ops as i16 + result.modifier as i16;
    let modifier = if result.modifier == 0 { String::new() } else { format!("{:+}", result.modifier) };
    let dice = format!("{side} {}+{}{modifier}={modified} vs {}", result.die, result.ops, result.target_number);
    let outcome = if !result.success() {
        format!("no better than {} — the coup fails in {country}", result.target_number)
    } else if result.added == 0 {
        format!("{side} removes {} {opponent} influence from {country}", result.removed)
    } else if result.removed == 0 {
        format!("{side} adds {} of its own influence to {country}", result.added)
    } else {
        format!(
            "{side} removes {} {opponent} influence from {country} and adds {} of its own",
            result.removed, result.added,
        )
    };
    format!("{dice} → {outcome}")
}

/// The map screens' one-line report of a just-closed operation — what
/// [`crate::game::Game::confirm`]/[`crate::game::Game::cancel`] returned,
/// and who acts next. The REPL's own `confirm`/`cancel` commands print
/// several kind-specific sentences instead; this is the one-row version
/// [`crate::interactive`] shows without leaving the map.
pub fn operation_closed_line(op: &crate::ops::Operation, committed: bool, next: crate::country::Superpower) -> String {
    if let crate::ops::Operation::Event(_) = op {
        return format!("{} event resolved — {next} to act", op.side());
    }
    let verdict = if committed { "confirmed" } else { "cancelled" };
    format!("{} {} {verdict} · {} of {} ops spent — {next} to act", op.side(), op.verb(), op.ops_spent(), op.ops_total())
}

/// The map screens' one-line report of what
/// [`crate::game::Game::abandon`] returned — always the same side still
/// to act, which is exactly the reassurance worth printing: unlike
/// [`operation_closed_line`], abandoning never hands the turn over, or
/// discards the card that funded it — that card is still in play,
/// ready for another `i`/`a`/`o`. A realignment or coup only ever
/// abandons with nothing spent (`abandon` refuses either the moment a
/// roll's been made); a placement can abandon with several points
/// pending, so those get their own wording naming what was undone rather
/// than claiming nothing happened.
pub fn operation_abandoned_line(op: &crate::ops::Operation) -> String {
    let spent = op.ops_spent();
    let detail = if spent == 0 { "nothing spent".to_string() } else { format!("{spent} of {} ops undone", op.ops_total()) };
    format!("{} {} abandoned — {detail}, card still in play", op.side(), op.verb())
}

/// Victory points, worded the way the dashboard header and the status bar
/// both show them — `"US +2"` / `"USSR +2"`, `"US +0"` at an even score —
/// pulled out so the two can't drift apart.
pub(crate) fn vp_line(vp: i8) -> String {
    if vp >= 0 {
        format!("US +{vp}")
    } else {
        format!("USSR +{}", -(vp as i16))
    }
}

/// The keys that play a card, open an operation with it, or pass the turn
/// — named by every view's no-operation hint. A region/world-map/country
/// screen only ever sees `Option<&Operation>`, never whether a card is
/// already in play, so this stays one card-agnostic sentence covering both
/// steps rather than two different hints the caller would have to choose
/// between. The status bar (which does know) uses its own three-state
/// wording instead — see `statusbar.rs`.
pub(crate) const BEGIN_HINT: &str = "space play card · i/a/o influence/realign/coup · s space race · t space track · e event · p pass";

/// One escape attempt, worded for the REPL: `Bear Trap: USSR discards Fidel, rolls 3 — escapes`.
pub fn trap_result_line(cards: &crate::cards::CardCatalog, r: &crate::game::TrapResult) -> String {
    format!(
        "{}: {} discards {}, rolls {} — {}",
        cards.card(r.trap).name,
        r.side,
        cards.card(r.discarded).name,
        r.roll,
        if r.escaped { "escapes" } else { "still trapped (1-4 escapes)" }
    )
}
