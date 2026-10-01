//! Keyboard-driven navigation for the terminal — raw mode, key events, the
//! alternate screen. This is the *only* place in the binary (and the whole
//! crate) that touches the terminal directly; every view it draws still
//! comes from `twilight_struggle::render`, which stays a pure `Canvas`
//! producer per its own module doc.

use std::collections::HashMap;
use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

use twilight_struggle::render::{
    coup_result_line, operation_abandoned_line, operation_closed_line, operation_header, render_card, render_country, render_hand,
    render_region, render_status_bar, render_world_map, roll_result_line, Canvas, HAND_ROWS,
};
use twilight_struggle::{
    CardCatalog, CardId, ColorMode, CountryId, Dice, Direction, Game, GameError, MapLayout, OperationKind, Region, RollOutcome, Superpower,
    ViewMode, WorldMap, CHINA_CARD,
};

/// Which screen is currently showing.
enum Screen {
    /// The to-scale world map, with `selected` picked by the arrow keys.
    World { selected: Region },
    /// One region's zoomed-in view, with its own country selection —
    /// `Esc` returns to `World` with the region still selected. `region`
    /// and `selected` always agree (`selected` is native to `region`):
    /// arrowing onto a guest chip — a country whose own region differs —
    /// immediately switches both fields to that country's own region
    /// instead ([`step_or_jump`]), so the whole map is walkable from here
    /// without ever leaving the region view.
    Region { region: Region, selected: CountryId },
    /// One country's detail screen, opened from `Region` with `Enter` or
    /// `r` — `Esc` returns to `Region` with the same country still
    /// selected. This is where a realignment roll or coup attempt
    /// actually happens: the full calculation is on screen above the key
    /// that resolves it, rather than firing straight from the region grid.
    /// Its own arrow keys follow a region-crossing jump the same way.
    Country { region: Region, selected: CountryId },
}

/// Puts the terminal into raw mode and the alternate screen, and — however
/// this function returns, including on panic — restores it. Without this,
/// a panic mid-render would leave the user's shell echoing nothing and
/// scrolled onto a screen they can't get back from.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = execute!(out, Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        let _ = out.flush();
    }
}

/// Drives the interactive world map until the user backs all the way out
/// (`Esc` from the world view, `q`, or Ctrl-C).
///
/// A status bar (`render::render_status_bar`) is drawn above every screen,
/// naming the turn/AR/active side/DEFCON/VP and the open operation's
/// balance — or, with none open, the keys that start one — since a
/// keypress alone carries no "USSR" the way the REPL prompt does.
///
/// A turn now starts by playing a card: `Space` plays the selected hand
/// card via [`Game::play_card`], and only then do `i`/`a`/`o` open an
/// influence placement, realignment, or coup — spending that card's own
/// ops — for the active side, from any of the three screens
/// ([`Game::begin`] always acts for [`Game::active`], so there's no side
/// to infer from the key itself; it's refused with no card in play). `p`
/// passes the active side's turn ([`Game::pass`]), refused the same way
/// while an operation is open or a card is in play, reported in the
/// message row via [`GameError`]'s own text. `c` confirms/closes the open
/// operation via [`Game::confirm`] and `X` cancels/closes it via
/// [`Game::cancel`] — either way discarding the card that funded it (or,
/// for the China Card, passing it face down to the opponent) and handing
/// the turn to the other side *without leaving the map*, so the newly
/// active side can immediately play its own card from the same screen.
///
/// Backspace steps back exactly one level, via whichever of
/// [`Game::abandon`] or [`Game::return_card`] applies: with an operation
/// open, it closes *that* (as long as nothing irreversible has happened —
/// see below), leaving the card in play so a different kind can be tried
/// without playing the card again; with no operation open but a card in
/// play, it puts the card back in the hand instead. Neither costs the
/// turn. An [`InfluencePlacement`](twilight_struggle::InfluencePlacement)
/// can always be abandoned this way, however many points are already
/// pending (they're simply discarded, like `X` would, minus the turn
/// cost), since placement never rolls a die and so never reveals anything
/// that can't be taken back; a [`Realignment`](twilight_struggle::Realignment)
/// or [`Coup`](twilight_struggle::Coup) can only be abandoned *before* its
/// first roll or attempt — the instant one's been made, `X`/`cancel` is
/// the only way out, and Backspace instead falls through to
/// [`GameError::CannotAbandon`]'s own text. For an
/// [`InfluencePlacement`](twilight_struggle::InfluencePlacement), `+`/`=`
/// places one point in the selected country (region or country screen) and
/// `u` undoes the last one — placement is undoable, so it never needs the
/// country screen's confirmation step. For a
/// [`Realignment`](twilight_struggle::Realignment) or a
/// [`Coup`](twilight_struggle::Coup), the region screen's `Enter`/`r` both
/// open the selected country's own detail screen instead of rolling
/// directly — that screen shows the full modifier/odds calculation, and
/// `r` there resolves the roll (or the coup's one attempt) — immediately
/// and permanently, since there's nothing to undo. `Esc`/`q` leave a
/// still-open session untouched rather than clearing it, so it can be
/// resumed from the REPL or by reopening the map.
///
/// The active side's hand is drawn as a strip below every screen
/// (`render::render_hand`), global like `i`/`a`/`o`/`p`/`Space`: `[`/`]`
/// cycle its selection and `z` toggles a zoomed detail overlay
/// (`render::render_card`, blitted onto the current screen's own canvas)
/// for the selected card — both no-ops on an empty hand, and `Space` is
/// too once a card is already in play (nothing left to select there to
/// play again). `Esc` closes an open zoom first, before whatever it would
/// otherwise do; `c`/`X`/`p` actually handing the turn over close it too,
/// since the newly active side's own hand takes its place. This still
/// isn't card *event* behaviour — nothing here reads a card's text or
/// triggers it, only its ops value, via [`Game::play_card`].
pub fn run(map: &WorldMap, layout: &MapLayout, cards: &CardCatalog, game: &mut Game, dice: &mut Dice, color: ColorMode) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut screen = Screen::World { selected: Region::Europe };
    // Remembers the last country selected in each region, so leaving a
    // region and coming back to it later re-selects the same one instead
    // of always resetting to its top-left-most country.
    let mut last_selected: HashMap<Region, CountryId> = HashMap::new();
    // The most recent refusal or roll outcome, shown on one extra row
    // below the canvas until the next key changes something. Cleared at
    // the top of every keypress *unless* `sticky` says to keep it — set
    // only by a roll's own outcome, the one message worth reading through
    // the next keypress since it's not recoverable from the screen the
    // way a refusal or a confirm/cancel/pass report is (the status bar
    // already reflects those). Kept here rather than threaded into
    // `render/`, which never touches the terminal or takes free-text
    // messages.
    let mut message: Option<String> = None;
    let mut sticky = false;
    // Each side's selected index into its own hand (China Card appended
    // last, when it holds it) — kept per side, indexed via `side_index`,
    // so passing the turn back and forth doesn't lose either side's place.
    let mut hand_selected: [usize; 2] = [0, 0];
    // Whether the selected card's full detail is overlaid on the current
    // screen. Survives ordinary navigation (the hand and its overlay
    // aren't tied to any one screen) but is always closed by an `Esc`
    // while it's open, and by `c`/`X`/`p` actually handing the turn over —
    // the newly active side's own hand takes its place.
    let mut zoomed = false;

    draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, color)?;
    loop {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                if !std::mem::take(&mut sticky) {
                    message = None;
                }
                match key.code {
                    KeyCode::Esc if zoomed => zoomed = false,
                    KeyCode::Esc if matches!(screen, Screen::World { .. }) => return Ok(()),
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('[') => cycle_hand(game, &mut hand_selected, -1),
                    KeyCode::Char(']') => cycle_hand(game, &mut hand_selected, 1),
                    KeyCode::Char('z') => {
                        if hand_item_count(game, game.active()) > 0 {
                            zoomed = !zoomed;
                        }
                    }
                    KeyCode::Char(' ') => {
                        if let Some(id) = selected_hand_card(game, &hand_selected) {
                            let side = game.active();
                            message = Some(match game.play_card(cards, id) {
                                Ok(()) => {
                                    zoomed = false;
                                    format!("{side} plays {} ({} ops) — i/a/o to use them", cards.card(id).name, cards.card(id).ops)
                                }
                                Err(e) => e.to_string(),
                            });
                        }
                    }
                    KeyCode::Char('i') => message = begin(game, OperationKind::Influence),
                    KeyCode::Char('a') => message = begin(game, OperationKind::Realign),
                    KeyCode::Char('o') => message = begin(game, OperationKind::Coup),
                    KeyCode::Char('p') => {
                        let passing = game.active();
                        message = Some(match game.pass() {
                            Ok(()) => {
                                zoomed = false;
                                format!("{passing} passes — {} to act", game.active())
                            }
                            Err(e) => e.to_string(),
                        });
                    }
                    KeyCode::Char('c') => {
                        message = Some(match game.confirm() {
                            Ok(op) => {
                                zoomed = false;
                                operation_closed_line(&op, true, game.active())
                            }
                            Err(e) => e.to_string(),
                        });
                    }
                    KeyCode::Char('X') => {
                        message = Some(match game.cancel() {
                            Ok(op) => {
                                zoomed = false;
                                operation_closed_line(&op, false, game.active())
                            }
                            Err(e) => e.to_string(),
                        });
                    }
                    KeyCode::Backspace => {
                        // Steps back exactly one level: an open operation
                        // closes first (leaving the card in play), and
                        // only once none is open does the card itself go
                        // back to the hand.
                        message = Some(if game.operation().is_some() {
                            match game.abandon() {
                                Ok(op) => operation_abandoned_line(&op),
                                Err(e) => e.to_string(),
                            }
                        } else {
                            match game.return_card() {
                                Ok(id) => format!("{} returned to hand", cards.card(id).name),
                                Err(e) => e.to_string(),
                            }
                        });
                    }
                    KeyCode::Char('u') => match game.undo(map) {
                        Ok(_) | Err(GameError::NothingToUndo) | Err(GameError::NoOperation) => {}
                        Err(e) => message = Some(e.to_string()),
                    },
                    _ => match &mut screen {
                        Screen::World { selected } => match key.code {
                            KeyCode::Left => *selected = selected.step(Direction::Left).unwrap_or(*selected),
                            KeyCode::Right => *selected = selected.step(Direction::Right).unwrap_or(*selected),
                            KeyCode::Up => *selected = selected.step(Direction::Up).unwrap_or(*selected),
                            KeyCode::Down => *selected = selected.step(Direction::Down).unwrap_or(*selected),
                            KeyCode::Enter => {
                                let region = *selected;
                                let country = last_selected
                                    .get(&region)
                                    .copied()
                                    .unwrap_or_else(|| layout.countries_in_region(map, region)[0]);
                                screen = Screen::Region { region, selected: country };
                            }
                            _ => {}
                        },
                        Screen::Region { region, selected } => match key.code {
                            KeyCode::Left => step_or_jump(map, layout, &mut last_selected, region, selected, Direction::Left),
                            KeyCode::Right => step_or_jump(map, layout, &mut last_selected, region, selected, Direction::Right),
                            KeyCode::Up => step_or_jump(map, layout, &mut last_selected, region, selected, Direction::Up),
                            KeyCode::Down => step_or_jump(map, layout, &mut last_selected, region, selected, Direction::Down),
                            KeyCode::Char('+') | KeyCode::Char('=') => {
                                if let Err(GameError::Placement(e)) = game.place(map, *selected) {
                                    message = Some(format!("{}: {e}", map.country(*selected).name));
                                }
                            }
                            KeyCode::Enter | KeyCode::Char('r') => {
                                screen = Screen::Country { region: *region, selected: *selected };
                            }
                            KeyCode::Esc => {
                                last_selected.insert(*region, *selected);
                                screen = Screen::World { selected: *region };
                            }
                            _ => {}
                        },
                        Screen::Country { region, selected } => match key.code {
                            KeyCode::Left => step_or_jump(map, layout, &mut last_selected, region, selected, Direction::Left),
                            KeyCode::Right => step_or_jump(map, layout, &mut last_selected, region, selected, Direction::Right),
                            KeyCode::Up => step_or_jump(map, layout, &mut last_selected, region, selected, Direction::Up),
                            KeyCode::Down => step_or_jump(map, layout, &mut last_selected, region, selected, Direction::Down),
                            KeyCode::Char('+') | KeyCode::Char('=') => {
                                if let Err(GameError::Placement(e)) = game.place(map, *selected) {
                                    message = Some(format!("{}: {e}", map.country(*selected).name));
                                }
                            }
                            KeyCode::Char('r') => {
                                let side = game.active();
                                match game.roll(map, *selected, dice) {
                                    Ok(RollOutcome::Realign(result)) => {
                                        message = Some(roll_result_line(map, side, &result));
                                        sticky = true;
                                    }
                                    Ok(RollOutcome::Coup(result)) => {
                                        message = Some(coup_result_line(map, side, &result));
                                        sticky = true;
                                    }
                                    Err(GameError::Realign(e)) => message = Some(format!("{}: {e}", map.country(*selected).name)),
                                    Err(GameError::Coup(e)) => message = Some(format!("{}: {e}", map.country(*selected).name)),
                                    Err(_) => {}
                                }
                            }
                            KeyCode::Esc => {
                                last_selected.insert(*region, *selected);
                                screen = Screen::Region { region: *region, selected: *selected };
                            }
                            _ => {}
                        },
                    },
                }
                draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, color)?;
            }
            Event::Resize(_, _) => draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, color)?,
            _ => {}
        }
    }
}

/// `i`/`a`/`o`: opens `kind` for the active side. `None` on success — the
/// status bar's own second row reports the new operation's balance the
/// moment `draw` runs next, so there's nothing this message would add —
/// `Some` naming the refusal ([`GameError`]'s own text) otherwise.
fn begin(game: &mut Game, kind: OperationKind) -> Option<String> {
    match game.begin(kind) {
        Ok(()) => Some(format!("started — {}", operation_header(game.operation().expect("begin just opened one")))),
        Err(e) => Some(e.to_string()),
    }
}

/// Indexes `hand_selected` by side — `Us` and `Ussr` each get their own
/// slot, so swapping the active side never loses the other side's place
/// in its own hand.
fn side_index(side: Superpower) -> usize {
    match side {
        Superpower::Us => 0,
        Superpower::Ussr => 1,
    }
}

/// How many cards `side`'s hand strip actually shows: its held cards,
/// plus the China Card when `side` currently holds it (see
/// [`twilight_struggle::Hands`]'s own doc for why that's counted
/// separately from [`Game::hand`]).
fn hand_item_count(game: &Game, side: Superpower) -> usize {
    let china = game.status().china_card == side;
    game.hand(side).len() + china as usize
}

/// The `CardId` the active side's current hand-strip selection refers to —
/// `hand[idx]` for an ordinary card, or the China Card once the selection
/// index runs past the held hand (`render_hand`'s own doc: the China Card
/// is always the strip's final slot). `None` on an empty hand, the one
/// case `Space`/`z` both already treat as a no-op.
fn selected_hand_card(game: &Game, hand_selected: &[usize; 2]) -> Option<CardId> {
    let side = game.active();
    let hand = game.hand(side);
    let count = hand_item_count(game, side);
    if count == 0 {
        return None;
    }
    let idx = hand_selected[side_index(side)].min(count - 1);
    Some(if idx < hand.len() { hand[idx] } else { CHINA_CARD })
}

/// `[`/`]`: moves the active side's own hand selection by `delta` (`-1` or
/// `1`), wrapping around either end. A no-op when that side's hand (plus a
/// possible China Card) is empty — there's nothing to select.
fn cycle_hand(game: &Game, hand_selected: &mut [usize; 2], delta: i32) {
    let side = game.active();
    let count = hand_item_count(game, side);
    if count == 0 {
        return;
    }
    let idx = side_index(side);
    let current = hand_selected[idx].min(count - 1) as i32;
    hand_selected[idx] = (current + delta).rem_euclid(count as i32) as usize;
}

/// Moves `selected` one step within `region`'s display grid via
/// [`MapLayout::step_country`]. If the step lands on a guest chip — a
/// country whose own region differs from `region` — the screen follows it
/// there: `last_selected` remembers where `region` was left from (the
/// same bookkeeping `Esc` already does, so coming back later re-selects
/// the country left behind), and `region` itself is rewritten to match,
/// keeping the invariant both `Screen::Region` and `Screen::Country` rely
/// on — that `selected` is always native to `region` — intact. Used by
/// both screens' arrow keys, so a jump behaves identically from either.
fn step_or_jump(map: &WorldMap, layout: &MapLayout, last_selected: &mut HashMap<Region, CountryId>, region: &mut Region, selected: &mut CountryId, dir: Direction) {
    let Some(next) = layout.step_country(map, *region, *selected, dir) else {
        return;
    };
    let next_region = map.country(next).region;
    if next_region != *region {
        last_selected.insert(*region, *selected);
        *region = next_region;
    }
    *selected = next;
}

/// Draws the status bar, the current screen (with a zoomed card's detail
/// blitted over it, if `zoomed`), the message row, and — pinned to the
/// bottom, always [`HAND_ROWS`] tall — the active side's hand strip.
/// Everything but the screen's own view is kept in full; the view is what
/// gets clipped from the bottom when the terminal is too short for all of
/// it, the same rule the status bar already followed before the hand
/// strip existed.
#[allow(clippy::too_many_arguments)]
fn draw(
    screen: &Screen,
    map: &WorldMap,
    layout: &MapLayout,
    cards: &CardCatalog,
    game: &Game,
    message: Option<&str>,
    hand_selected: &[usize; 2],
    zoomed: bool,
    color: ColorMode,
) -> io::Result<()> {
    let board = game.board();
    let op = game.operation();
    let mut canvas = match screen {
        Screen::World { selected } => render_world_map(map, layout, board, Some(*selected), op),
        Screen::Region { region, selected } => render_region(map, layout, board, *region, Some(*selected), op),
        Screen::Country { selected, .. } => render_country(map, layout, board, *selected, op, ViewMode::Interactive),
    };

    let side = game.active();
    let status = game.status();
    let china = (status.china_card == side).then_some(status.china_card_face_up);
    let hand = game.hand(side);
    let item_count = hand.len() + china.is_some() as usize;
    let selected_idx = (item_count > 0).then(|| hand_selected[side_index(side)].min(item_count - 1));
    let hand_canvas = render_hand(cards, hand, china, side, selected_idx);

    if zoomed && let Some(id) = selected_hand_card(game, hand_selected) {
        let china_face_up = (id == CHINA_CARD).then_some(status.china_card_face_up);
        let card_canvas = render_card(cards, id, china_face_up);
        // The overlay should never be clipped by a view too small for
        // it — widen the canvas first if it needs to be, rather than
        // letting `blit` silently cut the card off.
        if card_canvas.width() > canvas.width() || card_canvas.height() > canvas.height() {
            let mut widened = Canvas::new(canvas.width().max(card_canvas.width()), canvas.height().max(card_canvas.height()));
            widened.blit(&canvas, 0, 0);
            canvas = widened;
        }
        let row = canvas.height().saturating_sub(card_canvas.height()) / 2;
        let col = canvas.width().saturating_sub(card_canvas.width()) / 2;
        canvas.blit(&card_canvas, row, col);
    }

    let card_in_play = game.card_in_play().map(|id| cards.card(id));
    let bar = render_status_bar(layout, board, game.status(), card_in_play, op, canvas.width());

    let rows = terminal::size().map(|(_, h)| h as usize).unwrap_or(bar.height() + canvas.height() + HAND_ROWS);
    let view_budget = rows.saturating_sub(bar.height() + message.is_some() as usize + HAND_ROWS);

    let mut out = io::stdout();
    queue!(out, Clear(ClearType::All))?;
    let mut row = 0u16;
    let emit = |out: &mut io::Stdout, row: &mut u16, line: &str| -> io::Result<()> {
        if (*row as usize) < rows {
            queue!(out, MoveTo(0, *row))?;
            out.write_all(line.as_bytes())?;
            // Raw mode needs an explicit carriage return: a bare '\n'
            // only moves the cursor down a row, it doesn't return it to
            // column 0.
            out.write_all(b"\r\n")?;
        }
        *row += 1;
        Ok(())
    };

    for line in bar.render(color).split('\n') {
        emit(&mut out, &mut row, line)?;
    }
    for line in canvas.render(color).split('\n').take(view_budget) {
        emit(&mut out, &mut row, line)?;
    }
    if let Some(message) = message {
        emit(&mut out, &mut row, message)?;
    }
    for line in hand_canvas.render(color).split('\n') {
        emit(&mut out, &mut row, line)?;
    }
    out.flush()
}
