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

use twilight_struggle::render::{coup_result_line, render_country, render_region, render_world_map, roll_result_line};
use twilight_struggle::{ColorMode, CountryId, Dice, Direction, Game, GameError, MapLayout, Operation, Region, RollOutcome, ViewMode, WorldMap};

/// Which screen is currently showing.
enum Screen {
    /// The to-scale world map, with `selected` picked by the arrow keys.
    World { selected: Region },
    /// One region's zoomed-in view, with its own country selection —
    /// `Esc` returns to `World` with the region still selected.
    Region { region: Region, selected: CountryId },
    /// One country's detail screen, opened from `Region` with `Enter` or
    /// `r` — `Esc` returns to `Region` with the same country still
    /// selected. This is where a realignment roll or coup attempt
    /// actually happens: the full calculation is on screen above the key
    /// that resolves it, rather than firing straight from the region grid.
    Country { region: Region, selected: CountryId },
}

/// How the user left interactive mode, so the REPL can print a matching
/// line: leaving an operation open is not the same as confirming or
/// cancelling it.
pub enum Outcome {
    /// `Esc`/`q`/Ctrl-C. A still-open operation is left untouched — it
    /// stays in `Session.game`, resumable from the REPL or by reopening
    /// the map.
    Left,
    /// The open operation was confirmed, and the turn has passed to the
    /// other side.
    Confirmed,
    /// The open operation was cancelled, and the turn has passed to the
    /// other side.
    Cancelled,
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
/// `game`'s open operation, if any (a session can only be opened from the
/// REPL today), is the one in progress. `c` confirms/closes it — a
/// placement commits, a realignment's or coup's rolls are already on the
/// board — and hands the turn to the other side; `X` cancels/closes it,
/// also handing the turn over. For an
/// [`InfluencePlacement`](twilight_struggle::InfluencePlacement), `+`/`=`
/// places one point in the selected country (region or country screen) and
/// `u` undoes the last one — placement is undoable, so it never needs the
/// country screen's confirmation step. For a
/// [`Realignment`](twilight_struggle::Realignment) or a
/// [`Coup`](twilight_struggle::Coup), the region screen's `Enter`/`r` both
/// open the selected country's own detail screen instead of rolling
/// directly — that screen shows the full modifier/odds calculation, and
/// `r` there resolves the roll (or the coup's one attempt) — immediately
/// and permanently, since there's nothing to undo, and without ending the
/// turn (only `c`/`X` do that). Leaving via `Esc`/`q` keeps a still-open
/// session intact.
pub fn run(map: &WorldMap, layout: &MapLayout, game: &mut Game, dice: &mut Dice, color: ColorMode) -> io::Result<Outcome> {
    let _guard = TerminalGuard::enter()?;
    let mut screen = Screen::World { selected: Region::Europe };
    // Remembers the last country selected in each region, so leaving a
    // region and coming back to it later re-selects the same one instead
    // of always resetting to its top-left-most country.
    let mut last_selected: HashMap<Region, CountryId> = HashMap::new();
    // The most recent refusal or roll outcome, shown on one extra row
    // below the canvas until the next key changes something. Kept here
    // rather than threaded into `render/`, which never touches the
    // terminal or takes free-text messages. A roll's own result is
    // deliberately *not* cleared on every keypress the way a refusal is
    // (see below) — it's the one thing a player most wants to keep
    // reading after it appears.
    let mut message: Option<String> = None;

    draw(&screen, map, layout, game, message.as_deref(), color)?;
    loop {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(Outcome::Left);
                }
                // A roll's outcome stays on screen through the next key,
                // rather than being cleared like an ordinary refusal —
                // it's set fresh below whenever a new roll happens, and
                // any other key that would otherwise clear it doesn't
                // touch it.
                let rolled = matches!(key.code, KeyCode::Char('r'))
                    && matches!(screen, Screen::Country { .. })
                    && matches!(game.operation(), Some(Operation::Realign(_)) | Some(Operation::Coup(_)));
                if !rolled {
                    message = None;
                }
                match key.code {
                    KeyCode::Esc if matches!(screen, Screen::World { .. }) => return Ok(Outcome::Left),
                    KeyCode::Char('q') => return Ok(Outcome::Left),
                    KeyCode::Char('c') => {
                        if game.confirm().is_ok() {
                            return Ok(Outcome::Confirmed);
                        }
                    }
                    KeyCode::Char('X') => {
                        if game.cancel().is_ok() {
                            return Ok(Outcome::Cancelled);
                        }
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
                            _ => continue,
                        },
                        Screen::Region { region, selected } => match key.code {
                            KeyCode::Left => *selected = layout.step_country(map, *region, *selected, Direction::Left).unwrap_or(*selected),
                            KeyCode::Right => *selected = layout.step_country(map, *region, *selected, Direction::Right).unwrap_or(*selected),
                            KeyCode::Up => *selected = layout.step_country(map, *region, *selected, Direction::Up).unwrap_or(*selected),
                            KeyCode::Down => *selected = layout.step_country(map, *region, *selected, Direction::Down).unwrap_or(*selected),
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
                            _ => continue,
                        },
                        Screen::Country { region, selected } => match key.code {
                            KeyCode::Left => *selected = layout.step_country(map, *region, *selected, Direction::Left).unwrap_or(*selected),
                            KeyCode::Right => *selected = layout.step_country(map, *region, *selected, Direction::Right).unwrap_or(*selected),
                            KeyCode::Up => *selected = layout.step_country(map, *region, *selected, Direction::Up).unwrap_or(*selected),
                            KeyCode::Down => *selected = layout.step_country(map, *region, *selected, Direction::Down).unwrap_or(*selected),
                            KeyCode::Char('+') | KeyCode::Char('=') => {
                                if let Err(GameError::Placement(e)) = game.place(map, *selected) {
                                    message = Some(format!("{}: {e}", map.country(*selected).name));
                                }
                            }
                            KeyCode::Char('r') => {
                                let side = game.active();
                                match game.roll(map, *selected, dice) {
                                    Ok(RollOutcome::Realign(result)) => message = Some(roll_result_line(map, side, &result)),
                                    Ok(RollOutcome::Coup(result)) => message = Some(coup_result_line(map, side, &result)),
                                    Err(GameError::Realign(e)) => message = Some(format!("{}: {e}", map.country(*selected).name)),
                                    Err(GameError::Coup(e)) => message = Some(format!("{}: {e}", map.country(*selected).name)),
                                    Err(_) => {}
                                }
                            }
                            KeyCode::Esc => {
                                last_selected.insert(*region, *selected);
                                screen = Screen::Region { region: *region, selected: *selected };
                            }
                            _ => continue,
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

fn draw(screen: &Screen, map: &WorldMap, layout: &MapLayout, game: &Game, message: Option<&str>, color: ColorMode) -> io::Result<()> {
    let board = game.board();
    let op = game.operation();
    let canvas = match screen {
        Screen::World { selected } => render_world_map(map, layout, board, Some(*selected), op),
        Screen::Region { region, selected } => render_region(map, layout, board, *region, Some(*selected), op),
        Screen::Country { selected, .. } => render_country(map, layout, board, *selected, op, ViewMode::Interactive),
    };

    let rows = terminal::size().map(|(_, h)| h as usize).unwrap_or(canvas.height());

    let mut out = io::stdout();
    queue!(out, Clear(ClearType::All))?;
    let mut row = 0u16;
    for line in canvas.render(color).split('\n').take(rows) {
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
