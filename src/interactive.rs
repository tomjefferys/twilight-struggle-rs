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
    coup_result_line, operation_abandoned_line, operation_closed_line, operation_header, render_country, render_region,
    render_status_bar, render_world_map, roll_result_line,
};
use twilight_struggle::{
    ColorMode, CountryId, Dice, Direction, Game, GameError, MapLayout, OperationKind, Region, RollOutcome, ViewMode, WorldMap,
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
/// `i`/`a`/`o` open an influence placement, realignment, or coup for the
/// active side with a full turn's ops, from any of the three screens —
/// [`Game::begin`] always acts for [`Game::active`], so there's no side to
/// infer from the key itself. `p` passes the active side's turn
/// ([`Game::pass`]); both `begin` and `pass` are refused while an
/// operation is already open, reported in the message row via
/// [`GameError`]'s own text. `c` confirms/closes the open operation via
/// [`Game::confirm`] and `X` cancels/closes it via [`Game::cancel`] —
/// either way handing the turn to the other side *without leaving the
/// map*, so the newly active side can immediately press `i`/`a`/`o` on the
/// same screen. Backspace abandons the open operation via
/// [`Game::abandon`] instead — the free, no-turn-cost undo for opening the
/// wrong kind by mistake, as long as nothing irreversible has happened: an
/// [`InfluencePlacement`](twilight_struggle::InfluencePlacement) can
/// always be abandoned this way, however many points are already pending
/// (they're simply discarded, like `X` would, minus the turn cost), since
/// placement never rolls a die and so never reveals anything that can't be
/// taken back; a [`Realignment`](twilight_struggle::Realignment) or
/// [`Coup`](twilight_struggle::Coup) can only be abandoned *before* its
/// first roll or attempt — the instant one's been made, `X`/`cancel` is
/// the only way out. For an
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
pub fn run(map: &WorldMap, layout: &MapLayout, game: &mut Game, dice: &mut Dice, color: ColorMode) -> io::Result<()> {
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

    draw(&screen, map, layout, game, message.as_deref(), color)?;
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
                    KeyCode::Esc if matches!(screen, Screen::World { .. }) => return Ok(()),
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('i') => message = begin(game, OperationKind::Influence),
                    KeyCode::Char('a') => message = begin(game, OperationKind::Realign),
                    KeyCode::Char('o') => message = begin(game, OperationKind::Coup),
                    KeyCode::Char('p') => {
                        let passing = game.active();
                        message = Some(match game.pass() {
                            Ok(()) => format!("{passing} passes — {} to act", game.active()),
                            Err(e) => e.to_string(),
                        });
                    }
                    KeyCode::Char('c') => {
                        message = Some(match game.confirm() {
                            Ok(op) => operation_closed_line(&op, true, game.active()),
                            Err(e) => e.to_string(),
                        });
                    }
                    KeyCode::Char('X') => {
                        message = Some(match game.cancel() {
                            Ok(op) => operation_closed_line(&op, false, game.active()),
                            Err(e) => e.to_string(),
                        });
                    }
                    KeyCode::Backspace => {
                        message = Some(match game.abandon() {
                            Ok(op) => operation_abandoned_line(&op),
                            Err(e) => e.to_string(),
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
                draw(&screen, map, layout, game, message.as_deref(), color)?;
            }
            Event::Resize(_, _) => draw(&screen, map, layout, game, message.as_deref(), color)?,
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

fn draw(screen: &Screen, map: &WorldMap, layout: &MapLayout, game: &Game, message: Option<&str>, color: ColorMode) -> io::Result<()> {
    let board = game.board();
    let op = game.operation();
    let canvas = match screen {
        Screen::World { selected } => render_world_map(map, layout, board, Some(*selected), op),
        Screen::Region { region, selected } => render_region(map, layout, board, *region, Some(*selected), op),
        Screen::Country { selected, .. } => render_country(map, layout, board, *selected, op, ViewMode::Interactive),
    };
    let bar = render_status_bar(layout, board, game.status(), op, canvas.width());

    let rows = terminal::size().map(|(_, h)| h as usize).unwrap_or(bar.height() + canvas.height());
    // The status bar and the message row are the two rows worth keeping
    // when the whole thing is taller than the terminal: the view itself —
    // never the bar, never the message — is what gets clipped from the
    // bottom to make room.
    let view_budget = rows.saturating_sub(bar.height() + message.is_some() as usize);

    let mut out = io::stdout();
    queue!(out, Clear(ClearType::All))?;
    let lines = bar.render(color);
    let view = canvas.render(color);
    let mut row = 0u16;
    for line in lines.split('\n').chain(view.split('\n').take(view_budget)).take(rows) {
        queue!(out, MoveTo(0, row))?;
        out.write_all(line.as_bytes())?;
        // Raw mode needs an explicit carriage return: a bare '\n' only
        // moves the cursor down a row, it doesn't return it to column 0.
        out.write_all(b"\r\n")?;
        row += 1;
    }
    if let Some(message) = message
        && (row as usize) < rows
    {
        queue!(out, MoveTo(0, row))?;
        out.write_all(message.as_bytes())?;
        out.write_all(b"\r\n")?;
    }
    out.flush()
}
