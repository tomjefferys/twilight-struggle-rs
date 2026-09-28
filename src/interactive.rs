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

use twilight_struggle::render::{coup_result_line, render_region, render_world_map, roll_result_line};
use twilight_struggle::{Board, ColorMode, CountryId, Dice, Direction, MapLayout, Operation, Region, WorldMap};

/// Which screen is currently showing.
enum Screen {
    /// The to-scale world map, with `selected` picked by the arrow keys.
    World { selected: Region },
    /// One region's zoomed-in view, with its own country selection —
    /// `Esc` returns to `World` with the region still selected.
    Region { region: Region, selected: CountryId },
}

/// How the user left interactive mode, so the REPL can print a matching
/// line: leaving a placement session open is not the same as confirming
/// or discarding it.
pub enum Outcome {
    /// `Esc`/`q`/Ctrl-C. Any open placement is left untouched — it stays
    /// in `Session.placement`, resumable from the REPL or by reopening
    /// the map.
    Left,
    /// The open placement was committed to `board`.
    Confirmed,
    /// The open placement was discarded, unspent.
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
/// `op`, when `Some` on entry (or started implicitly never — a session
/// can only be opened from the REPL today), is an operation in progress.
/// `c` confirms/closes it into `board` (a placement commits; a
/// realignment's or coup's rolls are already there) and `X` cancels/closes
/// it. For an [`InfluencePlacement`](twilight_struggle::InfluencePlacement),
/// `+`/`=` places one point in the selected country (region screen only)
/// and `u` undoes the last one. For a
/// [`Realignment`](twilight_struggle::Realignment) or a
/// [`Coup`](twilight_struggle::Coup), `r` resolves a roll (or the coup's
/// one attempt) on the selected country (region screen only) —
/// immediately and permanently, since there's nothing to undo. Leaving
/// via `Esc`/`q` keeps a still-open session intact.
pub fn run(
    map: &WorldMap,
    layout: &MapLayout,
    board: &mut Board,
    op: &mut Option<Operation>,
    dice: &mut Dice,
    color: ColorMode,
) -> io::Result<Outcome> {
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

    draw(&screen, map, layout, board, op.as_ref(), message.as_deref(), color)?;
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
                    && matches!(screen, Screen::Region { .. })
                    && matches!(op, Some(Operation::Realign(_)) | Some(Operation::Coup(_)));
                if !rolled {
                    message = None;
                }
                match key.code {
                    KeyCode::Esc if matches!(screen, Screen::World { .. }) => return Ok(Outcome::Left),
                    KeyCode::Char('q') => return Ok(Outcome::Left),
                    KeyCode::Char('c') => {
                        if let Some(operation) = op.take() {
                            if let Operation::Influence(p) = operation {
                                *board = p.commit();
                            }
                            return Ok(Outcome::Confirmed);
                        }
                    }
                    KeyCode::Char('X') => {
                        if op.take().is_some() {
                            return Ok(Outcome::Cancelled);
                        }
                    }
                    KeyCode::Char('u') => {
                        match op.as_mut() {
                            Some(Operation::Influence(p)) => {
                                p.undo_last(map);
                            }
                            Some(Operation::Realign(_)) => {
                                message = Some("a resolved realignment roll can't be taken back".to_string());
                            }
                            Some(Operation::Coup(_)) => {
                                message = Some("a resolved coup can't be taken back".to_string());
                            }
                            None => {}
                        }
                    }
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
                                if let Some(Operation::Influence(p)) = op.as_mut()
                                    && let Err(e) = p.place(map, *selected)
                                {
                                    message = Some(format!("{}: {e}", map.country(*selected).name));
                                }
                            }
                            KeyCode::Char('r') => match op.as_mut() {
                                Some(Operation::Realign(r)) => {
                                    message = Some(match r.roll(map, board, *selected, dice) {
                                        Ok(result) => roll_result_line(map, r.side(), &result),
                                        Err(e) => format!("{}: {e}", map.country(*selected).name),
                                    });
                                }
                                Some(Operation::Coup(c)) => {
                                    message = Some(match c.attempt(map, board, *selected, dice) {
                                        Ok(result) => coup_result_line(map, c.side(), &result),
                                        Err(e) => format!("{}: {e}", map.country(*selected).name),
                                    });
                                }
                                _ => {}
                            },
                            KeyCode::Esc => {
                                last_selected.insert(*region, *selected);
                                screen = Screen::World { selected: *region };
                            }
                            _ => continue,
                        },
                    },
                }
                draw(&screen, map, layout, board, op.as_ref(), message.as_deref(), color)?;
            }
            Event::Resize(_, _) => draw(&screen, map, layout, board, op.as_ref(), message.as_deref(), color)?,
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    screen: &Screen,
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    op: Option<&Operation>,
    message: Option<&str>,
    color: ColorMode,
) -> io::Result<()> {
    let canvas = match screen {
        Screen::World { selected } => render_world_map(map, layout, board, Some(*selected), op),
        Screen::Region { region, selected } => render_region(map, layout, board, *region, Some(*selected), op),
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
