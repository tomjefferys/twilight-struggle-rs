//! The start menu behind the splash screen: a small pure state machine
//! ([`Menu`]) plus the terminal loop ([`run`]) that draws
//! `render::render_splash` centred and feeds it keys. Like `interactive.rs`
//! this is the only code that touches the terminal; the art is in `render/`.

use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::terminal::{self, Clear, ClearType};
use twilight_struggle::render::{render_splash, SplashItem, SplashMenu, SPLASH_WIDTH};
use twilight_struggle::ColorMode;

use crate::interactive::TerminalGuard;

/// Which side the human asked to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideChoice {
    Us,
    Ussr,
    Random,
}

/// What the menu resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuChoice {
    Resume,
    /// `human` is `None` for two players at one keyboard.
    NewGame { human: Option<SideChoice> },
    Repl,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    ChooseSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Resume,
    OnePlayer,
    TwoPlayers,
    Repl,
    Quit,
    Side(SideChoice),
    Back,
}

impl Entry {
    fn label(self) -> &'static str {
        match self {
            Entry::Resume => "Resume game",
            Entry::OnePlayer => "One player (vs AI)",
            Entry::TwoPlayers => "Two players (hotseat)",
            Entry::Repl => "Command console (REPL)",
            Entry::Quit => "Quit",
            Entry::Side(SideChoice::Us) => "Play the USA",
            Entry::Side(SideChoice::Ussr) => "Play the USSR",
            Entry::Side(SideChoice::Random) => "Random side",
            Entry::Back => "Back",
        }
    }
}

pub struct Menu {
    page: Page,
    selected: usize,
    can_resume: bool,
}

impl Menu {
    pub fn new(can_resume: bool) -> Self {
        Menu { page: Page::Main, selected: 0, can_resume }
    }

    fn entries(&self) -> Vec<Entry> {
        match self.page {
            Page::Main => {
                let mut v = Vec::new();
                if self.can_resume {
                    v.push(Entry::Resume);
                }
                v.extend([Entry::OnePlayer, Entry::TwoPlayers, Entry::Repl, Entry::Quit]);
                v
            }
            Page::ChooseSide => vec![Entry::Side(SideChoice::Us), Entry::Side(SideChoice::Ussr), Entry::Side(SideChoice::Random), Entry::Back],
        }
    }

    fn activate(&mut self, entry: Entry) -> Option<MenuChoice> {
        match entry {
            Entry::Resume => Some(MenuChoice::Resume),
            Entry::OnePlayer => {
                self.page = Page::ChooseSide;
                self.selected = 0;
                None
            }
            Entry::TwoPlayers => Some(MenuChoice::NewGame { human: None }),
            Entry::Repl => Some(MenuChoice::Repl),
            Entry::Quit => Some(MenuChoice::Quit),
            Entry::Side(side) => Some(MenuChoice::NewGame { human: Some(side) }),
            Entry::Back => {
                self.page = Page::Main;
                self.selected = 0;
                None
            }
        }
    }

    /// Feeds one key; returns the choice once one is made.
    pub fn key(&mut self, code: KeyCode) -> Option<MenuChoice> {
        let entries = self.entries();
        let n = entries.len();
        match code {
            KeyCode::Up => self.selected = (self.selected + n - 1) % n,
            KeyCode::Down | KeyCode::Tab => self.selected = (self.selected + 1) % n,
            KeyCode::Enter | KeyCode::Char(' ') => return self.activate(entries[self.selected]),
            KeyCode::Esc | KeyCode::Backspace => {
                if self.page == Page::ChooseSide {
                    self.page = Page::Main;
                    self.selected = 0;
                }
            }
            KeyCode::Char('q') => return Some(MenuChoice::Quit),
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let i = c.to_digit(10).unwrap() as usize;
                if i >= 1 && i <= n {
                    self.selected = i - 1;
                    return self.activate(entries[i - 1]);
                }
            }
            _ => {}
        }
        None
    }

    pub fn view(&self) -> SplashMenu {
        let title = match self.page {
            Page::Main => "Main menu",
            Page::ChooseSide => "Choose your side",
        };
        SplashMenu {
            title: title.to_string(),
            items: self
                .entries()
                .iter()
                .enumerate()
                .map(|(i, e)| SplashItem { label: format!("{}  {}", i + 1, e.label()), enabled: true })
                .collect(),
            selected: self.selected,
        }
    }
}

fn draw(menu: &Menu, color: ColorMode) -> io::Result<()> {
    let canvas = render_splash(&menu.view());
    let (cols, rows) = terminal::size().unwrap_or((SPLASH_WIDTH as u16, canvas.height() as u16));
    let left = (cols as usize).saturating_sub(SPLASH_WIDTH) / 2;
    let top = (rows as usize).saturating_sub(canvas.height()) / 2;
    let mut out = io::stdout();
    queue!(out, Clear(ClearType::All))?;
    for (i, line) in canvas.render(color).split('\n').enumerate() {
        if top + i >= rows as usize {
            break;
        }
        queue!(out, MoveTo(left as u16, (top + i) as u16))?;
        out.write_all(line.as_bytes())?;
    }
    out.flush()
}

/// Shows the menu until a choice is made. `can_resume` adds "Resume game".
pub fn run(color: ColorMode, can_resume: bool) -> io::Result<MenuChoice> {
    let _guard = TerminalGuard::enter()?;
    let mut menu = Menu::new(can_resume);
    draw(&menu, color)?;
    loop {
        match event::read()? {
            TermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(MenuChoice::Quit);
                }
                if let Some(choice) = menu.key(key.code) {
                    return Ok(choice);
                }
                draw(&menu, color)?;
            }
            TermEvent::Resize(..) => draw(&menu, color)?,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_wraps_and_enter_selects() {
        let mut m = Menu::new(false);
        assert_eq!(m.key(KeyCode::Up), None);
        assert_eq!(m.key(KeyCode::Enter), Some(MenuChoice::Quit));
    }

    #[test]
    fn resume_is_offered_only_when_there_is_a_game() {
        assert_eq!(Menu::new(false).entries()[0], Entry::OnePlayer);
        let mut m = Menu::new(true);
        assert_eq!(m.entries()[0], Entry::Resume);
        assert_eq!(m.key(KeyCode::Enter), Some(MenuChoice::Resume));
    }

    #[test]
    fn one_player_asks_for_a_side_and_esc_goes_back() {
        let mut m = Menu::new(false);
        assert_eq!(m.key(KeyCode::Char('1')), None);
        assert_eq!(m.page, Page::ChooseSide);
        m.key(KeyCode::Esc);
        assert_eq!(m.page, Page::Main);
        m.key(KeyCode::Enter);
        m.key(KeyCode::Down);
        assert_eq!(m.key(KeyCode::Enter), Some(MenuChoice::NewGame { human: Some(SideChoice::Ussr) }));
    }

    #[test]
    fn two_players_and_random_side() {
        assert_eq!(Menu::new(false).key(KeyCode::Char('2')), Some(MenuChoice::NewGame { human: None }));
        let mut m = Menu::new(false);
        m.key(KeyCode::Char('1'));
        assert_eq!(m.key(KeyCode::Char('3')), Some(MenuChoice::NewGame { human: Some(SideChoice::Random) }));
    }
}
