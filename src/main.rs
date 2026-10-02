use std::fs;
use std::io::{self, IsTerminal, Write};

use twilight_struggle::render::{
    coup_result_line, game_over_line, log_entry_line, log_text, operation_abandoned_line, operation_balance_line, render_card,
    render_country, render_hand, render_log, render_region, render_scoring_result, render_world, render_world_map, roll_result_line,
};
use twilight_struggle::{
    ai, CardCatalog, CardFound, ColorMode, Dice, EventOutcome, Found, Game, GameError, MapLayout, Operation, OperationKind, RandomAi,
    Region, RollOutcome, Scenario, Superpower, ViewMode, WorldMap, CHINA_CARD,
};

mod interactive;

struct Session {
    map: WorldMap,
    layout: MapLayout,
    cards: CardCatalog,
    /// Status, board, whichever card is in play, and whichever operation is
    /// open — see [`Game`]'s own doc for why these four move together.
    /// Turns are enforced entirely through this: a turn starts with `play`,
    /// which hands `influence`/`realign`/`coup` the card's own ops — none
    /// of the three take a side or an ops count of their own.
    game: Game,
    width: usize,
    color: ColorMode,
    /// False in one-shot mode, so `worldmap`/`wm` always falls back to a
    /// plain print there — the documented snapshot-regeneration workflow
    /// (`cargo run -- --color never worldmap`) runs on a TTY and must keep
    /// producing a single static render, never the interactive view.
    interactive_ok: bool,
    /// Rolls a realignment's or coup's dice. Seeded from entropy in the
    /// REPL, or from a fixed default (overridable with `--seed`) in
    /// one-shot mode, so the documented snapshot-regeneration workflow
    /// stays reproducible.
    dice: Dice,
    /// Picks the moves for `ai_side`'s turns, via [`ai::play_turn`]. Its own
    /// seed is independent of `dice`'s (derived from `--seed` when given,
    /// else entropy) so `--seed`/`seed <n>` keep controlling realignment
    /// and coup rolls only, not which moves the AI happens to pick.
    ai: RandomAi,
    /// Which side (if any) the AI plays instead of a human — at most one,
    /// so the REPL loop can never drive both sides without a human typing
    /// anything. `None` means every turn is typed at the prompt as usual.
    ai_side: Option<Superpower>,
}

fn main() {
    let map = WorldMap::standard().expect("standard map should be valid");
    let layout = MapLayout::standard(&map).expect("standard layout should be valid");
    let cards = CardCatalog::standard().expect("standard card catalog should be valid");
    let scenario = Scenario::demo(&map, &cards).expect("demo scenario should be valid");
    let game = Game::from_scenario(&scenario);

    let mut args = std::env::args().skip(1).peekable();
    let mut width = detect_width();
    let mut color = if std::env::var_os("NO_COLOR").is_some() {
        ColorMode::Never
    } else {
        ColorMode::Always
    };
    let mut seed: Option<u64> = None;
    let mut ai_side: Option<Superpower> = None;
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
            "--seed" => {
                if let Some(s) = args.next().and_then(|s| s.parse().ok()) {
                    seed = Some(s);
                }
            }
            "--ai" => {
                if let Some(side) = args.next().as_deref().and_then(parse_superpower) {
                    ai_side = Some(side);
                }
            }
            other => command_words.push(other.to_string()),
        }
    }

    let one_shot = !command_words.is_empty();
    // One-shot mode defaults to a fixed seed rather than entropy, so the
    // documented snapshot-regeneration workflow (`cargo run -- --color
    // never <command>`) stays reproducible; the REPL defaults to entropy
    // for real play. `--seed` overrides either.
    let dice = match seed {
        Some(s) => Dice::from_seed(s),
        None if one_shot => Dice::from_seed(0),
        None => Dice::from_entropy(),
    };
    // The AI's own seed is independent of `dice`'s (xored with a constant
    // salt rather than reused outright, so the two generators don't just
    // replay each other's sequence) — `--seed`/`seed <n>` keep controlling
    // realignment and coup rolls only, never which moves the AI picks.
    const AI_SEED_SALT: u64 = 0x41_495F_5345_4544;
    let ai = match seed {
        Some(s) => RandomAi::from_seed(s ^ AI_SEED_SALT),
        None if one_shot => RandomAi::from_seed(AI_SEED_SALT),
        None => RandomAi::from_entropy(),
    };

    let mut session = Session {
        map,
        layout,
        cards,
        game,
        width,
        color,
        interactive_ok: !one_shot,
        dice,
        ai,
        ai_side,
    };

    if one_shot {
        // One-shot mode: run a single command and exit, so scripts and
        // tests can invoke a view without driving the REPL.
        run_command(&mut session, &command_words.join(" "));
        return;
    }

    println!("Twilight Struggle — terminal map. Type `help` for commands, `quit` to exit.");
    maybe_run_ai_turn(&mut session);
    let stdin = io::stdin();
    loop {
        print!("{}", prompt(&session));
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
        maybe_run_ai_turn(&mut session);
    }
}

/// If `session.ai_side` names whoever's active right now, plays that turn
/// — a no-op otherwise (no AI side set, or it's the human's turn).
fn maybe_run_ai_turn(session: &mut Session) {
    if session.ai_side == Some(session.game.active()) {
        run_ai_turn(session);
    }
}

/// Plays the active side's current turn via [`ai::play_turn`] and echoes
/// every log entry it produced — the same lines `log` would show — so a
/// human watching the REPL sees what the AI just did without a separate
/// `log` call. Used both for automatic play (`maybe_run_ai_turn`, once
/// `session.ai_side` names the active side) and for the bare `ai` command,
/// which plays one turn regardless of `ai_side`.
fn run_ai_turn(session: &mut Session) {
    let side = session.game.active();
    println!("{side} (AI) plays:");
    let before = session.game.log().len();
    if let Err(e) = ai::play_turn(&mut session.ai, &mut session.game, &session.map, &session.cards, &mut session.dice) {
        println!("  AI error: {e}");
    }
    for entry in &session.game.log().entries()[before..] {
        println!("  {}", log_entry_line(&session.map, &session.cards, entry));
    }
}

/// `ai` plays the active side's turn once, right now, regardless of
/// whether auto-play is on. `ai us|ussr` turns auto-play on for that side
/// from now on (every subsequent turn of theirs plays itself, in both the
/// REPL and the interactive map); `ai off` turns it back off.
fn run_ai_command(session: &mut Session, words: &[&str]) {
    match words.get(1).copied() {
        None => run_ai_turn(session),
        Some("off") => {
            session.ai_side = None;
            println!("AI auto-play off");
        }
        Some(s) => match parse_superpower(s) {
            Some(side) => {
                session.ai_side = Some(side);
                println!("AI now plays {side} automatically");
            }
            None => println!("usage: ai [us|ussr|off]"),
        },
    }
}

/// A turn-aware prompt — e.g. `USSR AR 3/7 > ` or, with a card in play,
/// `USSR AR 3/7 [Fidel] > ` — so whose turn it is, and what's already been
/// played, never needs a separate `status` call to see.
fn prompt(session: &Session) -> String {
    let status = session.game.status();
    let card = match session.game.card_in_play() {
        Some(id) => format!("[{}] ", session.cards.card(id).name),
        None => String::new(),
    };
    format!("{} AR {}/{} {card}> ", session.game.active(), status.action_round, status.action_rounds_per_turn)
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
            print_operation_banner(session);
            let canvas = render_world(&session.map, &session.layout, session.game.view_board(), session.game.status(), session.width);
            println!("{}", canvas.render(session.color));
        }
        "worldmap" | "wm" => {
            if session.interactive_ok && io::stdin().is_terminal() && io::stdout().is_terminal() {
                // Turns can change hands any number of times inside the map
                // now — i/a/o/p/c/X all stay on screen — so there's nothing
                // left to report on return but the one state the next
                // prompt won't show on its own: a session left open.
                match interactive::run(
                    &session.map,
                    &session.layout,
                    &session.cards,
                    &mut session.game,
                    &mut session.dice,
                    session.color,
                    session.ai_side,
                    &mut session.ai,
                ) {
                    Ok(()) => {
                        if let Some(op) = session.game.operation() {
                            println!(
                                "left the map — {} {} still open ({} of {} ops left): confirm or cancel",
                                op.side(),
                                op.verb(),
                                op.remaining(),
                                op.ops_total(),
                            );
                        }
                    }
                    Err(e) => println!("interactive mode failed: {e}"),
                }
            } else {
                let canvas = render_world_map(&session.map, &session.layout, session.game.view_board(), None, session.game.operation());
                println!("{}", canvas.render(session.color));
            }
        }
        "region" => match words.get(1).and_then(|s| parse_region(s)) {
            Some(region) => {
                let canvas = render_region(&session.map, &session.layout, session.game.view_board(), region, None, session.game.operation());
                println!("{}", canvas.render(session.color));
            }
            None => println!("unknown region {:?}. Try: europe, asia, middleeast, africa, centralamerica, southamerica, or 1-6", words.get(1)),
        },
        "1" | "2" | "3" | "4" | "5" | "6" => {
            if let Some(region) = region_by_index(cmd.parse().unwrap()) {
                let canvas = render_region(&session.map, &session.layout, session.game.view_board(), region, None, session.game.operation());
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
            if let Some(op) = session.game.operation() {
                println!("finish or cancel the {} first ({} of {} ops left)", op.verb(), op.remaining(), op.ops_total());
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
                    let before = session.game.board().influence(id, superpower);
                    match cmd {
                        "set" => session.game.board_mut().set_influence(id, superpower, amount),
                        "add" => session.game.board_mut().add_influence(id, superpower, amount),
                        "remove" => session.game.board_mut().remove_influence(id, superpower, amount),
                        _ => unreachable!(),
                    }
                    let after = session.game.board().influence(id, superpower);
                    session.game.record_edit(id, superpower, before, after);
                    println!(
                        "{}  US {}  USSR {}",
                        session.map.country(id).name,
                        session.game.board().influence(id, Superpower::Us),
                        session.game.board().influence(id, Superpower::Ussr),
                    );
                }
                Found::None => println!("no country matches {country_query:?}"),
                Found::Ambiguous(ids) => print_ambiguous(session, &ids),
            }
        }
        "load" => {
            if let Some(op) = session.game.operation() {
                println!("finish or cancel the {} first ({} of {} ops left)", op.verb(), op.remaining(), op.ops_total());
                return;
            }
            if words.get(1) == Some(&"demo") {
                let scenario = Scenario::demo(&session.map, &session.cards).expect("demo scenario should be valid");
                session.game = Game::from_scenario(&scenario);
                session.game.record_note("loaded the demo scenario");
                println!("loaded demo scenario");
            } else {
                println!("usage: load demo");
            }
        }
        "play" => run_play_command(session, &words),
        "influence" => run_begin_command(session, OperationKind::Influence, &words),
        "realign" => run_begin_command(session, OperationKind::Realign, &words),
        "coup" => run_begin_command(session, OperationKind::Coup, &words),
        "event" => run_event_command(session),
        "place" => run_place_command(session, &words),
        "roll" => run_roll_command(session, &words),
        "undo" => run_undo_command(session),
        "confirm" => run_confirm_command(session),
        "cancel" => run_cancel_command(session),
        "abandon" => run_abandon_command(session),
        "status" => run_status_command(session),
        "pass" => run_pass_command(session),
        "ai" => run_ai_command(session, &words),
        "hand" => run_hand_command(session, &words),
        "card" => run_card_command(session, &words),
        "log" | "history" => run_log_command(session, &words),
        "export" => run_export_command(session, &words),
        "seed" => match words.get(1).and_then(|s| s.parse().ok()) {
            Some(s) => {
                session.dice = Dice::from_seed(s);
                println!("dice reseeded from {s}");
            }
            None => println!("usage: seed <n>"),
        },
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
            let canvas = render_country(&session.map, &session.layout, session.game.view_board(), id, session.game.operation(), ViewMode::Static);
            println!("{}", canvas.render(session.color));
        }
        Found::None => match session.layout.find_by_code(query) {
            Some(id) => {
                let canvas =
                    render_country(&session.map, &session.layout, session.game.view_board(), id, session.game.operation(), ViewMode::Static);
                println!("{}", canvas.render(session.color));
            }
            None => println!("no country matches {query:?}"),
        },
        Found::Ambiguous(ids) => print_ambiguous(session, &ids),
    }
}

/// Printed above the six-region dashboard, the one view left with no room
/// of its own to show pending operation state — the region, world map,
/// and country views all show this same information inline instead (the
/// country view's own Operation panel is exactly this line, split across
/// its title and first row). Reads the committed board, not the
/// speculative one — the line only names the touched countries and
/// remaining ops, both already tracked by the operation itself.
fn print_operation_banner(session: &Session) {
    if let Some(op) = session.game.operation() {
        println!("{}", operation_balance_line(&session.layout, session.game.board(), op));
    }
}

fn print_ambiguous(session: &Session, ids: &[twilight_struggle::CountryId]) {
    let names: Vec<&str> = ids.iter().map(|&id| session.map.country(id).name.as_str()).collect();
    println!("ambiguous: {}", names.join(", "));
}

/// `play <id|name>` takes a card from the active side's hand (or, for the
/// China Card, from wherever it currently sits face up) and makes it the
/// card in play — the first step of a turn now, since `influence`/
/// `realign`/`coup` all need a card's ops to open with. The same forgiving
/// lookup `card` uses (`CardCatalog::find`).
fn run_play_command(session: &mut Session, words: &[&str]) {
    let Some(query) = words.get(1..).map(|w| w.join(" ")) else {
        println!("usage: play <id|name>");
        return;
    };
    if query.is_empty() {
        println!("usage: play <id|name>");
        return;
    }
    match session.cards.find(&query) {
        CardFound::One(id) => {
            let side = session.game.active();
            match session.game.play_card(&session.cards, id) {
                Ok(()) => println!(
                    "{side} plays {} ({} ops) — influence/realign/coup to use them",
                    session.cards.card(id).name,
                    session.cards.card(id).ops,
                ),
                Err(e) => println!("{e}"),
            }
        }
        CardFound::None => println!("no card matches {query:?}"),
        CardFound::Ambiguous(ids) => {
            let names: Vec<&str> = ids.iter().map(|&id| session.cards.card(id).name.as_str()).collect();
            println!("ambiguous: {}", names.join(", "));
        }
    }
}

/// `influence`/`realign`/`coup` open an operation for the active side,
/// spending whichever card's already in play — no arguments, since
/// enforcing turns means there's nothing left for a caller to name.
/// Refused if a session is already open, or no card has been played yet
/// (`play` first), per [`Game::begin`].
fn run_begin_command(session: &mut Session, kind: OperationKind, words: &[&str]) {
    if words.len() != 1 {
        println!("{} takes no arguments now — it always starts for {}, the active side, spending the card already in play", words[0], session.game.active());
        return;
    }
    match session.game.begin(kind) {
        Ok(()) => {
            let op = session.game.operation().expect("begin just opened one");
            let side = op.side();
            let ops = op.ops_total();
            match kind {
                OperationKind::Influence => println!("started an influence placement for {side} with {ops} ops"),
                OperationKind::Realign => {
                    println!("started a {side} realignment with {ops} ops — each roll resolves immediately onto the board and cannot be taken back")
                }
                OperationKind::Coup => println!(
                    "started a {side} coup with {ops} ops — `roll <country>` spends all {ops} on one attempt, \
                     resolved immediately onto the board and cannot be taken back"
                ),
            }
        }
        Err(e) => println!("{e}"),
    }
}

/// `event` resolves whichever card is in play's own text — the other
/// thing a played card can fund, alongside `influence`/`realign`/`coup`.
/// Only a scoring card's event is implemented so far; anything else is
/// refused with [`GameError::EventNotImplemented`]'s own text. Prints the
/// same breakdown the interactive map's modal shows, and, if the event
/// just won the game, [`game_over_line`] right after it.
fn run_event_command(session: &mut Session) {
    match session.game.play_event(&session.map, &session.cards) {
        Ok(EventOutcome::Scoring(result)) => {
            let vp_after = session.game.status().vp;
            let canvas = render_scoring_result(&session.map, &session.cards, &result, vp_after, None);
            println!("{}", canvas.render(session.color));
            if let Some(victory) = session.game.winner() {
                println!("{}", game_over_line(victory));
            }
        }
        Err(e) => println!("{e}"),
    }
}

/// `place <country> [n]` places `n` (default 1) points of influence, one
/// at a time, stopping and reporting on the first one that's refused.
fn run_place_command(session: &mut Session, words: &[&str]) {
    match session.game.operation() {
        Some(Operation::Influence(_)) => {}
        Some(op) => {
            println!("a {} session is open, not a placement — `place` only works while placing influence", op.verb());
            return;
        }
        None => {
            println!("no placement session open. Start one with: influence");
            return;
        }
    }
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
        match session.game.place(&session.map, id) {
            Ok(cost) => {
                let name = &session.map.country(id).name;
                let placement = session.game.placement().expect("place just succeeded on an open placement");
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

/// `roll <country>` resolves one realignment roll, or a coup's one
/// attempt, immediately — whichever kind of session is open. There's no
/// "current target" to default to, so the country is required. Doesn't
/// advance the turn; only `confirm`/`cancel`/`pass` do that.
fn run_roll_command(session: &mut Session, words: &[&str]) {
    let rest = &words[1..];
    if rest.is_empty() {
        println!("usage: roll <country>");
        return;
    }
    let country_query = rest.join(" ");
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
    let side = session.game.active();
    match session.game.roll(&session.map, id, &mut session.dice) {
        Ok(RollOutcome::Realign(result)) => println!("{}", roll_result_line(&session.map, side, &result)),
        Ok(RollOutcome::Coup(result)) => println!("{}", coup_result_line(&session.map, side, &result)),
        Err(GameError::Realign(e)) => println!("cannot roll in {}: {e}", session.map.country(id).name),
        Err(GameError::Coup(e)) => println!("cannot coup {}: {e}", session.map.country(id).name),
        Err(GameError::WrongKind { open }) => {
            println!("a {open} session is open, not a realignment or coup — `roll` only works during one of those")
        }
        Err(GameError::NoOperation) => println!("no realignment or coup session open. Start one with: realign or coup"),
        Err(e) => println!("{e}"),
    }
}

/// `hand [us|ussr]` prints the named side's hand (default: the active
/// side), with the China Card appended if that side currently holds it —
/// a static, unselected print, the same view the interactive map's hand
/// strip draws with a selection.
fn run_hand_command(session: &Session, words: &[&str]) {
    let side = match words.get(1) {
        Some(s) => match parse_superpower(s) {
            Some(side) => side,
            None => {
                println!("expected 'us' or 'ussr', got {s:?}");
                return;
            }
        },
        None => session.game.active(),
    };
    let status = session.game.status();
    let china = (status.china_card == side).then_some(status.china_card_face_up);
    // Only the active side can have a card in play, so a named inactive
    // side's hand never shows one.
    let in_play = (side == session.game.active()).then(|| session.game.card_in_play_slot()).flatten();
    let canvas = render_hand(&session.cards, session.game.hand(side), china, side, None, in_play);
    println!("{}", canvas.render(session.color));
}

/// `card <id|name>` prints one card's full detail — the same zoom view
/// the interactive map overlays on `z`.
fn run_card_command(session: &Session, words: &[&str]) {
    let Some(query) = words.get(1..).map(|w| w.join(" ")) else {
        println!("usage: card <id|name>");
        return;
    };
    if query.is_empty() {
        println!("usage: card <id|name>");
        return;
    }
    match session.cards.find(&query) {
        CardFound::One(id) => {
            let status = session.game.status();
            let china_face_up = (id == CHINA_CARD).then_some(status.china_card_face_up);
            let canvas = render_card(&session.cards, id, china_face_up);
            println!("{}", canvas.render(session.color));
        }
        CardFound::None => println!("no card matches {query:?}"),
        CardFound::Ambiguous(ids) => {
            let names: Vec<&str> = ids.iter().map(|&id| session.cards.card(id).name.as_str()).collect();
            println!("ambiguous: {}", names.join(", "));
        }
    }
}

fn run_undo_command(session: &mut Session) {
    match session.game.undo(&session.map) {
        Ok(id) => {
            let total = session.game.placement().map_or(0, |p| p.ops_total());
            println!("undid a point in {} — {} of {total} ops left", session.map.country(id).name, session.game.ops_available());
        }
        Err(e) => println!("{e}"),
    }
}

fn run_confirm_command(session: &mut Session) {
    match session.game.confirm() {
        Ok(Operation::Influence(placement)) => {
            let side = placement.side();
            let spent = placement.ops_spent();
            let total = placement.ops_total();
            let summary = placement
                .pending_countries()
                .iter()
                .map(|&(id, n)| format!("{} +{n}", session.map.country(id).name))
                .collect::<Vec<_>>()
                .join(", ");
            if spent == total {
                println!("committed {spent} ops for {side}: {summary}");
            } else {
                println!("committed {spent} of {total} ops for {side} ({} unspent): {summary}", total - spent);
            }
        }
        Ok(Operation::Realign(realignment)) => {
            let side = realignment.side();
            let spent = realignment.ops_spent();
            let total = realignment.ops_total();
            let rolls = realignment.history().len();
            if spent == total {
                println!("done — {side} spent all {total} ops on {rolls} roll(s), already resolved on the board");
            } else {
                println!(
                    "done — {side} spent {spent} of {total} ops on {rolls} roll(s) ({} unspent), already resolved on the board",
                    total - spent
                );
            }
        }
        Ok(Operation::Coup(coup)) => {
            let side = coup.side();
            let total = coup.ops_total();
            match coup.result() {
                Some(result) => println!(
                    "done — {side} spent all {total} ops couping {}, already resolved on the board",
                    session.map.country(result.target).name,
                ),
                None => println!("closed — {side}'s coup never attempted; all {total} ops are simply lost"),
            }
        }
        Err(e) => {
            println!("{e}");
            return;
        }
    }
    println!("{} to act", session.game.active());
}

fn run_cancel_command(session: &mut Session) {
    match session.game.cancel() {
        Ok(Operation::Influence(placement)) => {
            println!(
                "cancelled — discarded {} pending marker(s), {} ops returned",
                placement.pending_countries().iter().map(|&(_, n)| n as u32).sum::<u32>(),
                placement.ops_spent(),
            );
        }
        Ok(Operation::Realign(realignment)) => {
            println!(
                "closed — {} roll(s) already resolved on the board can't be undone; {} unspent ops are simply lost",
                realignment.history().len(),
                realignment.remaining(),
            );
        }
        Ok(Operation::Coup(coup)) => match coup.result() {
            Some(result) => println!(
                "closed — the coup on {} already resolved on the board and can't be undone",
                session.map.country(result.target).name,
            ),
            None => println!("closed — the coup never attempted; all {} ops are simply lost", coup.ops_total()),
        },
        Err(e) => {
            println!("{e}");
            return;
        }
    }
    println!("{} to act", session.game.active());
}

/// `abandon` steps back exactly one level, the same cascade Backspace
/// drives in the interactive map: with an operation open, it closes
/// *that* — for free, as long as nothing irreversible has happened (a
/// placement can always be abandoned, however many points are pending,
/// since it never rolls a die; a realignment or coup can only be abandoned
/// before its first roll or attempt — `cancel` is the only way out once
/// one's been made) — leaving the card in play. With no operation open but
/// a card in play, it puts the card back in the hand instead.
fn run_abandon_command(session: &mut Session) {
    if session.game.operation().is_some() {
        match session.game.abandon() {
            Ok(op) => println!("{}", operation_abandoned_line(&op)),
            Err(e) => println!("{e}"),
        }
    } else {
        match session.game.return_card() {
            Ok(id) => println!("{} returned to hand", session.cards.card(id).name),
            Err(e) => println!("{e}"),
        }
    }
}

/// Reports whose turn it is, the turn/action-round counters, whichever
/// card is in play, and — if one's open — the balance of the current
/// operation. Takes over the reporting role bare `influence`/`realign`/
/// `coup` used to have, now that those always start a new operation
/// instead.
fn run_status_command(session: &Session) {
    let status = session.game.status();
    let card = match session.game.card_in_play() {
        Some(id) => format!("{} in play", session.cards.card(id).name),
        None => "no card in play".to_string(),
    };
    println!(
        "TURN {}   AR {}/{}   {} to act   {card}   {} ops available",
        status.turn,
        status.action_round,
        status.action_rounds_per_turn,
        session.game.active(),
        session.game.ops_available(),
    );
    print_operation_banner(session);
    if let Some(victory) = session.game.winner() {
        println!("{}", game_over_line(victory));
    }
}

/// `pass` forfeits the active side's turn without opening an operation.
/// Refused while one is already open — cancel it first.
fn run_pass_command(session: &mut Session) {
    let passing = session.game.active();
    match session.game.pass() {
        Ok(()) => println!("{passing} passes — {} to act", session.game.active()),
        Err(e) => println!("{e}"),
    }
}

/// `log` prints the whole history, colour-banded by side; `log <n>` shows
/// only the last `n` entries. Reads the log at any time, including mid-
/// operation — the in-progress operation simply isn't in it yet, which is
/// correct, since nothing about it is final until `confirm`/`cancel`.
fn run_log_command(session: &Session, words: &[&str]) {
    if session.game.log().is_empty() {
        println!("no history yet");
        return;
    }
    let tail = words.get(1).and_then(|s| s.parse().ok());
    let canvas = render_log(&session.map, &session.cards, session.game.log(), tail);
    println!("{}", canvas.render(session.color));
}

/// `export <path>` writes the whole history to `path` as plain text —
/// exactly what `log` shows, minus colour. Refuses nothing: the log is
/// readable (and exportable) regardless of whether an operation is open.
fn run_export_command(session: &Session, words: &[&str]) {
    let Some(&path) = words.get(1) else {
        println!("usage: export <path>");
        return;
    };
    let mut text = log_text(&session.map, &session.cards, session.game.log());
    text.push('\n');
    match fs::write(path, text) {
        Ok(()) => println!("wrote {} entries to {path}", session.game.log().len()),
        Err(e) => println!("could not write {path}: {e}"),
    }
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
                          (and the same arrow-key selection continues inside a
                          region; Enter there opens that country's own detail
                          screen — also where a realignment roll or coup
                          attempt actually happens, with the calculation on
                          screen above the key that resolves it). A status
                          bar at the top of every screen names the turn, AR,
                          active side, DEFCON, VP, and either the card in
                          play (and the open operation's balance, once one
                          is) or a prompt to play one. The active side's
                          hand is drawn below every screen; [ and ] cycle
                          the selected card, z zooms it into a full detail
                          overlay, space plays it
  region <name>, 1-6      zoom into one region (europe/asia/middleeast/africa/centralamerica/southamerica)
  country <name>, /<name> a single country's detail, with all its neighbours (name or code)
  set <c> <us|ussr> <n>   set a country's influence
  add <c> <us|ussr> <n>   add influence (saturates)
  remove <c> <us|ussr> <n> remove influence (saturates at 0)
  load demo               reload the bundled demo scenario

  status                  whose turn it is, the turn/AR counters, whichever
                          card is in play, and the open operation's
                          balance, if any
  pass                    forfeit the active side's turn — refused with a
                          card in play (return it first) or an operation
                          open — or press p inside the interactive map
  ai                      have the AI play the active side's current turn,
                          once, choosing uniformly among whatever
                          Game::legal_actions lists right now
  ai us|ussr              from now on, the AI plays that side's turns
                          automatically (in the REPL and the interactive
                          map alike) — `--ai us|ussr` sets this at launch
  ai off                  turn automatic AI play back off

  play <id|name>          take a card from the active side's hand (id,
                          name prefix, or exact name — the China Card
                          included, when it's held face up) — its ops
                          value is what influence/realign/coup spend next,
                          or `event` resolves its text instead (a scoring
                          card has no ops, so event is the only way to
                          play one) — or press space inside the
                          interactive map

  influence               start placing influence for the active side,
                          spending the card already in play — or press i
                          inside the interactive map
  place <country> [n]     place n influence (default 1); 1 op, or 2 in an
                          opponent-controlled country — refused if there's
                          no influence there, in a neighbour, or a border
                          with your own superpower
  undo                    take back the last point placed, refunding it

  realign                 start realigning for the active side, spending
                          the card already in play; view a country
                          (country <name>, region, or worldmap) to see the
                          modifier and odds breakdown first — or press a
                          inside the interactive map
  roll <country>          resolve one realignment roll for 1 op — resolves
                          immediately onto the board and CANNOT be undone
  undo                    (during a realignment) refuses: rolls can't be
                          taken back, only placement points can

  coup                    start a coup for the active side, spending the
                          card already in play; view a country to see its
                          target number (stability ×2) and success odds
                          first — or press o inside the interactive map
  roll <country>          resolve the coup's one attempt, spending all the
                          card's ops at once — resolves immediately onto
                          the board and CANNOT be undone; a second `roll`
                          in the same session is refused
  undo                    (during a coup) refuses: a resolved attempt
                          can't be taken back

  event                   resolve the card in play's own text instead of
                          its ops — so far, only the seven scoring cards'
                          events are implemented (anything else is
                          refused); discards the card (or, for a card
                          removed after its event, removes it from the
                          game entirely) and passes the turn — or, if it
                          wins the game outright (VP reaching ±20, or
                          Europe Scoring's Control tier), ends it instead
                          — or press e inside the interactive map

  confirm                 commit a pending placement, or close a finished
                          realignment or coup (whose rolls are already on
                          the board) — either way, the card played is
                          discarded (the China Card instead passes face
                          down to the opponent) and the turn passes to the
                          other side; unspent ops are simply lost
  cancel                  discard an unconfirmed placement, unspent; or
                          close a realignment or coup, leaving any rolls
                          already made in place — either way the card
                          played is discarded the same way `confirm` does,
                          and the turn passes to the other side
  abandon                 step back exactly one level, for free — no turn
                          cost: with an operation open, close *that* (as
                          long as nothing irreversible has happened — a
                          placement can always abandon, however many
                          points are pending, since they're just
                          discarded; a realignment or coup can only
                          abandon before its first roll or attempt —
                          cancel is the only way out once one's been made),
                          leaving the card in play; with no operation open
                          but a card in play, return the card to the hand
                          instead
                          (while a session is open: map/world show a balance
                          banner, region/country show it inline; all three
                          of worldmap/region/country mark touched countries
                          and are navigable inside interactive mode — space
                          plays the selected card, i/a/o start an operation
                          with it, e resolves its event (a scoring card's
                          only way to play), and p passes, +/= place on
                          region or country, u undo, c confirm, X cancel,
                          Backspace steps back one level the same way
                          `abandon` does — c and X hand the turn over
                          without leaving the map, so the next side can
                          start its own card from the same screen; r rolls
                          or attempts a coup only on the country screen,
                          opened from region with Enter or r; set/add/
                          remove/load are refused until you confirm or
                          cancel; once a scoring event ends the game, the
                          status bar's own row says so and every further
                          action is refused)

  hand [us|ussr]          the named side's hand (default: active side),
                          as a strip of mini-card boxes — or see it drawn
                          under every screen inside the interactive map
  card <id|name>          one card's full detail (id, name prefix, or
                          exact name) — or press z to zoom the selected
                          card inside the interactive map

  log [n], history        the game's history so far (or just the last n
                          entries), colour-banded by side; a realignment
                          or coup roll appears the instant it resolves, a
                          placement as one entry when it's confirmed or
                          cancelled
  export <path>           write the history to a file as plain text —
                          exactly what `log` shows, minus colour

  seed <n>                reseed the dice (for reproducible play)
  width <n>               set the render width
  color on|off            toggle ANSI colour
  help, ?                 this text
  quit, exit, q           leave"
    );
}
