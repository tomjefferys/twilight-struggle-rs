//! Keyboard-driven navigation for the terminal — raw mode, key events, the
//! alternate screen. This is the *only* place in the binary (and the whole
//! crate) that touches the terminal directly; every view it draws still
//! comes from `twilight_struggle::render`, which stays a pure `Canvas`
//! producer per its own module doc.

use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

use twilight_struggle::render::{render_region, render_world_map};
use twilight_struggle::{Board, ColorMode, Direction, MapLayout, Region, WorldMap};

/// Which screen is currently showing.
enum Screen {
    /// The to-scale world map, with `selected` picked by the arrow keys.
    World { selected: Region },
    /// One region's zoomed-in view — `Esc` returns to `World` with the
    /// same region still selected.
    Region(Region),
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
pub fn run(map: &WorldMap, layout: &MapLayout, board: &Board, color: ColorMode) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut screen = Screen::World { selected: Region::Europe };

    draw(&screen, map, layout, board, color)?;
    loop {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                match &mut screen {
                    Screen::World { selected } => match key.code {
                        KeyCode::Left => *selected = selected.step(Direction::Left).unwrap_or(*selected),
                        KeyCode::Right => *selected = selected.step(Direction::Right).unwrap_or(*selected),
                        KeyCode::Up => *selected = selected.step(Direction::Up).unwrap_or(*selected),
                        KeyCode::Down => *selected = selected.step(Direction::Down).unwrap_or(*selected),
                        KeyCode::Enter => screen = Screen::Region(*selected),
                        KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                        _ => continue,
                    },
                    Screen::Region(region) => match key.code {
                        KeyCode::Esc => screen = Screen::World { selected: *region },
                        KeyCode::Char('q') => return Ok(()),
                        _ => continue,
                    },
                }
                draw(&screen, map, layout, board, color)?;
            }
            Event::Resize(_, _) => draw(&screen, map, layout, board, color)?,
            _ => {}
        }
    }
}

fn draw(screen: &Screen, map: &WorldMap, layout: &MapLayout, board: &Board, color: ColorMode) -> io::Result<()> {
    let canvas = match screen {
        Screen::World { selected } => render_world_map(map, layout, board, Some(*selected)),
        Screen::Region(region) => render_region(map, layout, board, *region),
    };

    let rows = terminal::size().map(|(_, h)| h as usize).unwrap_or(canvas.height());

    let mut out = io::stdout();
    queue!(out, Clear(ClearType::All))?;
    for (i, line) in canvas.render(color).split('\n').take(rows).enumerate() {
        queue!(out, MoveTo(0, i as u16))?;
        out.write_all(line.as_bytes())?;
        // Raw mode needs an explicit carriage return: a bare '\n' only
        // moves the cursor down a row, it doesn't return it to column 0.
        out.write_all(b"\r\n")?;
    }
    out.flush()
}
