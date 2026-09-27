use std::io::{self, IsTerminal, Write};

use twilight_struggle::render::{render_country, render_region, render_world, render_world_map};
use twilight_struggle::{Board, ColorMode, Found, MapLayout, Region, Scenario, Superpower, WorldMap};

mod interactive;

struct Session {
    map: WorldMap,
    layout: MapLayout,
    board: Board,
    scenario: Scenario,
    width: usize,
    color: ColorMode,
    /// False in one-shot mode, so `worldmap`/`wm` always falls back to a
    /// plain print there — the documented snapshot-regeneration workflow
    /// (`cargo run -- --color never worldmap`) runs on a TTY and must keep
    /// producing a single static render, never the interactive view.
    interactive_ok: bool,
}

fn main() {
    let map = WorldMap::standard().expect("standard map should be valid");
    let layout = MapLayout::standard(&map).expect("standard layout should be valid");
    let scenario = Scenario::demo(&map).expect("demo scenario should be valid");
    let board = scenario.board.clone();

    let mut args = std::env::args().skip(1).peekable();
    let mut width = detect_width();
    let mut color = if std::env::var_os("NO_COLOR").is_some() {
        ColorMode::Never
    } else {
        ColorMode::Always
    };
    let mut command_words = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--width" => {
                if let Some(w) = args.next().and_then(|s| s.parse().ok()) {
                    width = w;
                }
            }
            "--color" => match args.next().as_deref() {
                Some("always") => color = ColorMode::Always,
                Some("never") => color = ColorMode::Never,
                Some("auto") | None => {}
                Some(_) => {}
            },
            other => command_words.push(other.to_string()),
        }
    }

    let mut session = Session {
        map,
        layout,
        board,
        scenario,
        width,
        color,
        interactive_ok: command_words.is_empty(),
    };

    if !command_words.is_empty() {
        // One-shot mode: run a single command and exit, so scripts and
        // tests can invoke a view without driving the REPL.
        run_command(&mut session, &command_words.join(" "));
        return;
    }

    println!("Twilight Struggle — terminal map. Type `help` for commands, `quit` to exit.");
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "quit" | "exit" | "q") {
            break;
        }
        run_command(&mut session, line);
    }
}

fn detect_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(100)
}

fn run_command(session: &mut Session, line: &str) {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(&cmd) = words.first() else { return };

    match cmd {
        "map" | "world" => {
            let canvas = render_world(&session.map, &session.layout, &session.board, &session.scenario.status, session.width);
            println!("{}", canvas.render(session.color));
        }
        "worldmap" | "wm" => {
            if session.interactive_ok && io::stdin().is_terminal() && io::stdout().is_terminal() {
                if let Err(e) = interactive::run(&session.map, &session.layout, &session.board, session.color) {
                    println!("interactive mode failed: {e}");
                }
            } else {
                let canvas = render_world_map(&session.map, &session.layout, &session.board, None);
                println!("{}", canvas.render(session.color));
            }
        }
        "region" => match words.get(1).and_then(|s| parse_region(s)) {
            Some(region) => {
                let canvas = render_region(&session.map, &session.layout, &session.board, region);
                println!("{}", canvas.render(session.color));
            }
            None => println!("unknown region {:?}. Try: europe, asia, middleeast, africa, centralamerica, southamerica, or 1-6", words.get(1)),
        },
        "1" | "2" | "3" | "4" | "5" | "6" => {
            if let Some(region) = region_by_index(cmd.parse().unwrap()) {
                let canvas = render_region(&session.map, &session.layout, &session.board, region);
                println!("{}", canvas.render(session.color));
            }
        }
        "country" => {
            let Some(query) = words.get(1..).map(|w| w.join(" ")) else {
                println!("usage: country <name>");
                return;
            };
            print_country(session, &query);
        }
        "set" | "add" | "remove" => {
            if words.len() < 4 {
                println!("usage: {cmd} <country> <us|ussr> <amount>");
                return;
            }
            let country_query = words[1];
            let Some(superpower) = parse_superpower(words[2]) else {
                println!("expected 'us' or 'ussr', got {:?}", words[2]);
                return;
            };
            let Ok(amount) = words[3].parse::<u8>() else {
                println!("expected a number, got {:?}", words[3]);
                return;
            };
            match session.map.find(country_query) {
                Found::One(id) => {
                    match cmd {
                        "set" => session.board.set_influence(id, superpower, amount),
                        "add" => session.board.add_influence(id, superpower, amount),
                        "remove" => session.board.remove_influence(id, superpower, amount),
                        _ => unreachable!(),
                    }
                    println!(
                        "{}  US {}  USSR {}",
                        session.map.country(id).name,
                        session.board.influence(id, Superpower::Us),
                        session.board.influence(id, Superpower::Ussr),
                    );
                }
                Found::None => println!("no country matches {country_query:?}"),
                Found::Ambiguous(ids) => print_ambiguous(session, &ids),
            }
        }
        "load" => {
            if words.get(1) == Some(&"demo") {
                session.scenario = Scenario::demo(&session.map).expect("demo scenario should be valid");
                session.board = session.scenario.board.clone();
                println!("loaded demo scenario");
            } else {
                println!("usage: load demo");
            }
        }
        "width" => match words.get(1).and_then(|s| s.parse().ok()) {
            Some(w) => {
                session.width = w;
                println!("width set to {w}");
            }
            None => println!("usage: width <n>"),
        },
        "color" => match words.get(1) {
            Some(&"on") => session.color = ColorMode::Always,
            Some(&"off") => session.color = ColorMode::Never,
            _ => println!("usage: color on|off"),
        },
        "help" | "?" => print_help(),
        _ if cmd.starts_with('/') => print_country(session, &cmd[1..]),
        _ => println!("unknown command {cmd:?}. Type `help` for commands."),
    }
}

fn print_country(session: &Session, query: &str) {
    match session.map.find(query) {
        Found::One(id) => {
            let canvas = render_country(&session.map, &session.board, id);
            println!("{}", canvas.render(session.color));
        }
        Found::None => match session.layout.find_by_code(query) {
            Some(id) => {
                let canvas = render_country(&session.map, &session.board, id);
                println!("{}", canvas.render(session.color));
            }
            None => println!("no country matches {query:?}"),
        },
        Found::Ambiguous(ids) => print_ambiguous(session, &ids),
    }
}

fn print_ambiguous(session: &Session, ids: &[twilight_struggle::CountryId]) {
    let names: Vec<&str> = ids.iter().map(|&id| session.map.country(id).name.as_str()).collect();
    println!("ambiguous: {}", names.join(", "));
}

fn parse_superpower(s: &str) -> Option<Superpower> {
    match s.to_lowercase().as_str() {
        "us" | "usa" => Some(Superpower::Us),
        "ussr" | "su" | "soviet" => Some(Superpower::Ussr),
        _ => None,
    }
}

fn parse_region(s: &str) -> Option<Region> {
    match s.to_lowercase().replace([' ', '-', '_'], "").as_str() {
        "europe" => Some(Region::Europe),
        "asia" => Some(Region::Asia),
        "middleeast" => Some(Region::MiddleEast),
        "africa" => Some(Region::Africa),
        "centralamerica" => Some(Region::CentralAmerica),
        "southamerica" => Some(Region::SouthAmerica),
        _ => None,
    }
}

fn region_by_index(n: u8) -> Option<Region> {
    match n {
        1 => Some(Region::Europe),
        2 => Some(Region::Asia),
        3 => Some(Region::MiddleEast),
        4 => Some(Region::Africa),
        5 => Some(Region::CentralAmerica),
        6 => Some(Region::SouthAmerica),
        _ => None,
    }
}

fn print_help() {
    println!(
        "\
Commands:
  map, world              the six-region dashboard
  worldmap, wm            the whole world as one geographic map (codes, no names);
                          arrow keys select a region, Enter zooms in, Esc backs out
  region <name>, 1-6      zoom into one region (europe/asia/middleeast/africa/centralamerica/southamerica)
  country <name>, /<name> a single country's detail, with all its neighbours (name or code)
  set <c> <us|ussr> <n>   set a country's influence
  add <c> <us|ussr> <n>   add influence (saturates)
  remove <c> <us|ussr> <n> remove influence (saturates at 0)
  load demo               reload the bundled demo scenario
  width <n>               set the render width
  color on|off            toggle ANSI colour
  help, ?                 this text
  quit, exit, q           leave"
    );
}
