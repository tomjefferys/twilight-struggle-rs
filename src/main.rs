use std::io::{self, IsTerminal, Write};

use twilight_struggle::render::{placement_balance_line, render_country, render_region, render_world, render_world_map};
use twilight_struggle::{Board, ColorMode, Found, InfluencePlacement, MapLayout, Region, Scenario, Superpower, WorldMap};

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
    /// An uncommitted influence placement, if `ops` has started one.
    /// While it's open, every view renders its speculative board instead
    /// of `board`, and `set`/`add`/`remove`/`load` are refused so the
    /// board a placement was judged legal against can't shift underneath
    /// it.
    placement: Option<InfluencePlacement>,
}

/// The board every view should read: the placement's speculative one
/// while a session is open, otherwise the committed board.
fn view_board(session: &Session) -> &Board {
    session.placement.as_ref().map_or(&session.board, InfluencePlacement::board)
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
        placement: None,
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
            print_placement_banner(session);
            let canvas = render_world(&session.map, &session.layout, view_board(session), &session.scenario.status, session.width);
            println!("{}", canvas.render(session.color));
        }
        "worldmap" | "wm" => {
            if session.interactive_ok && io::stdin().is_terminal() && io::stdout().is_terminal() {
                match interactive::run(&session.map, &session.layout, &mut session.board, &mut session.placement, session.color) {
                    Ok(interactive::Outcome::Left) => {
                        if let Some(p) = &session.placement {
                            println!(
                                "left the map — {} placement still open ({} of {} ops left): confirm or cancel",
                                p.side(),
                                p.remaining(),
                                p.ops_total(),
                            );
                        }
                    }
                    Ok(interactive::Outcome::Confirmed) => println!("placement confirmed"),
                    Ok(interactive::Outcome::Cancelled) => println!("placement cancelled"),
                    Err(e) => println!("interactive mode failed: {e}"),
                }
            } else {
                let canvas = render_world_map(&session.map, &session.layout, view_board(session), None, session.placement.as_ref());
                println!("{}", canvas.render(session.color));
            }
        }
        "region" => match words.get(1).and_then(|s| parse_region(s)) {
            Some(region) => {
                let canvas = render_region(&session.map, &session.layout, view_board(session), region, None, session.placement.as_ref());
                println!("{}", canvas.render(session.color));
            }
            None => println!("unknown region {:?}. Try: europe, asia, middleeast, africa, centralamerica, southamerica, or 1-6", words.get(1)),
        },
        "1" | "2" | "3" | "4" | "5" | "6" => {
            if let Some(region) = region_by_index(cmd.parse().unwrap()) {
                let canvas = render_region(&session.map, &session.layout, view_board(session), region, None, session.placement.as_ref());
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
            if let Some(p) = &session.placement {
                println!("finish or cancel the influence placement first ({} of {} ops left)", p.remaining(), p.ops_total());
                return;
            }
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
            if let Some(p) = &session.placement {
                println!("finish or cancel the influence placement first ({} of {} ops left)", p.remaining(), p.ops_total());
                return;
            }
            if words.get(1) == Some(&"demo") {
                session.scenario = Scenario::demo(&session.map).expect("demo scenario should be valid");
                session.board = session.scenario.board.clone();
                println!("loaded demo scenario");
            } else {
                println!("usage: load demo");
            }
        }
        "ops" => run_ops_command(session, &words),
        "place" => run_place_command(session, &words),
        "undo" => run_undo_command(session),
        "confirm" => run_confirm_command(session),
        "cancel" => run_cancel_command(session),
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
            print_placement_banner(session);
            let canvas = render_country(&session.map, view_board(session), id);
            println!("{}", canvas.render(session.color));
        }
        Found::None => match session.layout.find_by_code(query) {
            Some(id) => {
                print_placement_banner(session);
                let canvas = render_country(&session.map, view_board(session), id);
                println!("{}", canvas.render(session.color));
            }
            None => println!("no country matches {query:?}"),
        },
        Found::Ambiguous(ids) => print_ambiguous(session, &ids),
    }
}

/// Printed above any view that has no room of its own to show pending
/// placement state (the dashboard, a single country) — the region and
/// world map views show this same line in their own footer instead.
fn print_placement_banner(session: &Session) {
    if let Some(p) = &session.placement {
        println!("{}", placement_balance_line(&session.layout, p));
    }
}

fn print_ambiguous(session: &Session, ids: &[twilight_struggle::CountryId]) {
    let names: Vec<&str> = ids.iter().map(|&id| session.map.country(id).name.as_str()).collect();
    println!("ambiguous: {}", names.join(", "));
}

/// `ops <us|ussr> <n>` starts a placement session; bare `ops` reports the
/// open one, if any.
fn run_ops_command(session: &mut Session, words: &[&str]) {
    if words.len() == 1 {
        match &session.placement {
            Some(_) => print_placement_banner(session),
            None => println!("no placement session open. Start one with: ops <us|ussr> <n>"),
        }
        return;
    }
    if let Some(p) = &session.placement {
        println!(
            "a placement session is already open ({} of {} ops left) — confirm or cancel it first",
            p.remaining(),
            p.ops_total()
        );
        return;
    }
    if words.len() != 3 {
        println!("usage: ops <us|ussr> <n>");
        return;
    }
    let Some(side) = parse_superpower(words[1]) else {
        println!("expected 'us' or 'ussr', got {:?}", words[1]);
        return;
    };
    let Ok(ops) = words[2].parse::<u8>() else {
        println!("expected a number, got {:?}", words[2]);
        return;
    };
    if ops == 0 {
        println!("ops must be at least 1");
        return;
    }
    session.placement = Some(InfluencePlacement::new(side, ops, &session.board));
    println!("started an influence placement for {side} with {ops} ops");
}

/// `place <country> [n]` places `n` (default 1) points of influence, one
/// at a time, stopping and reporting on the first one that's refused.
fn run_place_command(session: &mut Session, words: &[&str]) {
    let Some(placement) = &mut session.placement else {
        println!("no placement session open. Start one with: ops <us|ussr> <n>");
        return;
    };
    let rest = &words[1..];
    if rest.is_empty() {
        println!("usage: place <country> [n]");
        return;
    }
    // A country name can be several words (East Germany, South Africa,
    // ...), so only the trailing word — and only when the name still has
    // something left without it — is ever read as the optional count.
    let (country_words, amount) = match rest.split_last() {
        Some((&last, init)) if !init.is_empty() && last.parse::<u8>().is_ok() => (init, last.parse::<u8>().unwrap()),
        _ => (rest, 1),
    };
    if amount == 0 {
        println!("expected a positive number, got \"0\"");
        return;
    }
    let country_query = country_words.join(" ");
    let id = match session.map.find(&country_query) {
        Found::One(id) => id,
        Found::None => {
            println!("no country matches {country_query:?}");
            return;
        }
        Found::Ambiguous(ids) => {
            print_ambiguous(session, &ids);
            return;
        }
    };
    for placed in 0..amount {
        match placement.place(&session.map, id) {
            Ok(cost) => {
                let name = &session.map.country(id).name;
                println!(
                    "{name}  US {}  USSR {}  (pending +{}, cost {cost}, {} of {} ops left)",
                    placement.board().influence(id, Superpower::Us),
                    placement.board().influence(id, Superpower::Ussr),
                    placement.pending(id),
                    placement.remaining(),
                    placement.ops_total(),
                );
            }
            Err(e) => {
                if placed > 0 {
                    println!("placed {placed} of {amount} in {}: {e}", session.map.country(id).name);
                } else {
                    println!("cannot place in {}: {e}", session.map.country(id).name);
                }
                return;
            }
        }
    }
}

fn run_undo_command(session: &mut Session) {
    let Some(placement) = &mut session.placement else {
        println!("no placement session open");
        return;
    };
    match placement.undo_last(&session.map) {
        Some(id) => println!(
            "undid a point in {} — {} of {} ops left",
            session.map.country(id).name,
            placement.remaining(),
            placement.ops_total(),
        ),
        None => println!("nothing to undo"),
    }
}

fn run_confirm_command(session: &mut Session) {
    let Some(placement) = session.placement.take() else {
        println!("no placement session open");
        return;
    };
    let side = placement.side();
    let spent = placement.ops_spent();
    let total = placement.ops_total();
    let summary = placement
        .pending_countries()
        .iter()
        .map(|&(id, n)| format!("{} +{n}", session.map.country(id).name))
        .collect::<Vec<_>>()
        .join(", ");
    session.board = placement.commit();
    if spent == total {
        println!("committed {spent} ops for {side}: {summary}");
    } else {
        println!("committed {spent} of {total} ops for {side} ({} unspent): {summary}", total - spent);
    }
}

fn run_cancel_command(session: &mut Session) {
    let Some(placement) = session.placement.take() else {
        println!("no placement session open");
        return;
    };
    println!(
        "cancelled — discarded {} pending marker(s), {} ops returned",
        placement.pending_countries().iter().map(|&(_, n)| n as u32).sum::<u32>(),
        placement.ops_spent(),
    );
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
                          (and the same arrow-key selection continues inside a region)
  region <name>, 1-6      zoom into one region (europe/asia/middleeast/africa/centralamerica/southamerica)
  country <name>, /<name> a single country's detail, with all its neighbours (name or code)
  set <c> <us|ussr> <n>   set a country's influence
  add <c> <us|ussr> <n>   add influence (saturates)
  remove <c> <us|ussr> <n> remove influence (saturates at 0)
  load demo               reload the bundled demo scenario

  ops <us|ussr> <n>       start placing influence with n operation points
  ops                     show the open session's ops balance
  place <country> [n]     place n influence (default 1); 1 op, or 2 in an
                          opponent-controlled country — refused if there's
                          no influence there, in a neighbour, or a border
                          with your own superpower
  undo                    take back the last point placed, refunding it
  confirm                 commit the pending placement to the board
  cancel                  discard it, unspent
                          (while a session is open: map/world/country show
                          a balance banner; worldmap/region mark pending
                          countries with a + and are navigable the same
                          way inside interactive mode — + place, u undo,
                          c confirm, X cancel; set/add/remove/load are
                          refused until you confirm or cancel)

  width <n>               set the render width
  color on|off            toggle ANSI colour
  help, ?                 this text
  quit, exit, q           leave"
    );
}
