use std::fs;
use std::io::{self, IsTerminal};

use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::Editor;

use twilight_struggle::render::{
    coup_result_line, game_over_line, log_entry_line, log_text, piles_text, render_final_scoring, PileTab, ongoing_effect_line, operation_abandoned_line, operation_balance_line, render_card,
    render_country, render_event_result, render_military_track, render_space_result, render_tracks, TrackTab, render_space_track, render_war_result, render_hand, render_log, render_region, render_scoring_result, render_world, render_world_map, roll_result_line,
};
use twilight_struggle::{
    ai, CardCatalog, CardFound, CardId, ColorMode, Dice, EventOutcome, Found, Game, GameError, GameStatus, MapLayout, Operation,
    OperationKind, Region, RollOutcome, Scenario, StateLibrary, Superpower, ViewMode, WorldMap, CHINA_CARD, DEFCON_RANGE,
    MAX_HAND_SIZE, TURN_RANGE,
};

use completion::TsHelper;

mod completion;
mod interactive;

/// Every first-word command the REPL recognises, for the line editor's
/// completion ([`completion::candidates`]) — kept in step with
/// [`print_help`]'s own text by
/// `tests::every_command_is_mentioned_in_help`, rather than generating
/// one from the other (the help text's prose doesn't reduce to a flat
/// list, and a const doesn't carry the per-command explanation).
/// Deliberately excludes the single-letter/numeric shortcuts (`q`,
/// `1`-`6`, `?`) and `quit`/`exit`, which the REPL loop handles before
/// `run_command` ever sees them.
const COMMANDS: &[&str] = &[
    "map", "world", "worldmap", "wm", "region", "country", "set", "add", "remove", "clear", "blank", "load", "save", "states", "play",
    "influence", "realign", "coup", "event", "place", "roll", "undo", "confirm", "cancel", "abandon", "status", "pass", "ai", "hand",
    "card", "log", "history", "export", "seed", "width", "color", "debug", "vp", "defcon", "turn", "ar", "active", "china", "give",
    "discard", "exile", "help", "+", "-", "take", "mode", "space", "spacerace", "milops", "tracks", "track", "escape", "defuse", "piles", "headline", "new",
];

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
    ai: Box<dyn ai::Ai>,
    /// Which kind of AI `ai` is, for `ai kind` to report and rebuild.
    ai_kind: ai::AiKind,
    /// Which side (if any) the AI plays instead of a human — at most one,
    /// so the REPL loop can never drive both sides without a human typing
    /// anything. `None` means every turn is typed at the prompt as usual.
    ai_side: Option<Superpower>,
    /// Whether debug-mode state editing (`set`/`add`/`remove`/`clear`/
    /// `vp`/`defcon`/`turn`/`ar`/`active`/`china`/`give`/`discard`/
    /// `exile`/`blank`) is unlocked — off by default, so a normal game
    /// can't be nudged out of true by a stray command, and shown in the
    /// prompt (`[debug]`) so it's never silently on. Loading a named test
    /// state (`load <file>/<name>`, as opposed to `load demo`) turns this
    /// on automatically, since that's exactly what a test state is for.
    debug: bool,
    /// The named test-state library (`data/states/`) `load`/`save`/
    /// `states` read and write — see [`StateLibrary`]'s own doc for the
    /// file format and why it's read from disk rather than embedded.
    states: StateLibrary,
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
    let mut state_ref: Option<String> = None;
    let mut new_game_flag = false;
    let mut ai_kind = ai::AiKind::default();
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
            "--state" => {
                state_ref = args.next();
            }
            "--new" => new_game_flag = true,
            "--ai-kind" => match args.next().as_deref().and_then(ai::AiKind::parse) {
                Some(kind) => ai_kind = kind,
                None => eprintln!("--ai-kind takes heuristic or random"),
            },
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
    let ai_seed = match seed {
        Some(s) => Some(s ^ AI_SEED_SALT),
        None if one_shot => Some(AI_SEED_SALT),
        None => None,
    };
    let ai = ai_kind.build(ai_seed);

    // Built before `map`/`cards` move into `session`, for the line
    // editor's completer (`completion::TsHelper`) below.
    let country_names: Vec<String> = map.iter().map(|(_, c)| c.name.clone()).collect();
    let card_names: Vec<String> = cards.iter().map(|c| c.name.clone()).collect();

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
        ai_kind,
        ai_side,
        debug: false,
        states: StateLibrary::standard(),
    };

    if new_game_flag {
        session.game = Game::new_game(&session.map, &session.cards, &mut session.dice);
        session.game.record_note("new game");
    }
    if let Some(reference) = &state_ref {
        run_load_state_command(&mut session, reference);
    }

    if one_shot {
        // One-shot mode: run a single command and exit, so scripts and
        // tests can invoke a view without driving the REPL.
        run_command(&mut session, &command_words.join(" "));
        return;
    }

    println!("Twilight Struggle — terminal map. Type `help` for commands, `quit` to exit.");
    maybe_run_ai_turn(&mut session);

    let mut rl = Editor::<TsHelper, DefaultHistory>::new().expect("rustyline editor should initialize");
    rl.set_helper(Some(TsHelper {
        commands: COMMANDS.iter().map(|s| s.to_string()).collect(),
        cards: card_names,
        countries: country_names,
        states: StateLibrary::standard(),
    }));

    loop {
        match rl.readline(&prompt(&session)) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                rl.add_history_entry(line).ok();
                if matches!(line, "quit" | "exit" | "q") {
                    break;
                }
                run_command(&mut session, line);
                maybe_run_ai_turn(&mut session);
            }
            // Before rustyline, Ctrl-C had no handler at all and fell
            // through to the terminal's own default SIGINT behaviour,
            // which killed the process outright — rustyline instead
            // catches it and hands it back as this error, so matching
            // bash's "clear the line" convention here would silently
            // take away the exit keystroke everyone's muscle memory
            // already relies on. Keep it an exit.
            Err(ReadlineError::Interrupted) => break,
            Err(ReadlineError::Eof) => break,
            Err(e) => {
                println!("readline error: {e}");
                break;
            }
        }
    }
}

/// If `session.ai_side` names whoever's active right now, plays that turn
/// — a no-op otherwise (no AI side set, or it's the human's turn).
fn maybe_run_ai_turn(session: &mut Session) {
    // What the last action round set off (NORAD) needs the map to settle.
    if session.game.settlement_due() {
        let before = session.game.log().len();
        session.game.settle(&session.map, &session.cards, &mut session.dice);
        let entries = &session.game.log().entries()[before..];
        for (i, entry) in entries.iter().enumerate() {
            println!("{}", log_entry_line(&session.map, &session.cards, entry));
            if let twilight_struggle::Event::FinalScoring { results, china, vp_after } = &entry.event {
                let winner = match entries.get(i + 1).map(|e| &e.event) {
                    Some(twilight_struggle::Event::GameOver(victory)) => Some(*victory),
                    _ => None,
                };
                println!("{}", render_final_scoring(results, *china, *vp_after, winner, None).render(session.color));
            }
        }
        if session.game.operation().is_some() {
            print_event_prompt(session);
        }
    }
    // `decider`, not `active`: an event's chooser is the card's own side.
    // A few rounds, since one human move can hand the AI an event to
    // resolve *and* then its own turn straight after.
    for _ in 0..3 {
        if session.ai_side != Some(session.game.decider()) || session.game.winner().is_some() {
            return;
        }
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
    let side = session.game.decider();
    println!("{side} (AI) plays:");
    let before = session.game.log().len();
    if let Err(e) = ai::play_turn(session.ai.as_mut(), &mut session.game, &session.map, &session.cards, &mut session.dice) {
        println!("  AI error: {e}");
    }
    for entry in &session.game.log().entries()[before..] {
        println!("  {}", log_entry_line(&session.map, &session.cards, entry));
    }
}

/// Swaps in a fresh AI of `kind`, seeded from the clock.
fn set_ai_kind(session: &mut Session, kind: ai::AiKind) {
    session.ai = kind.build(None);
    session.ai_kind = kind;
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
        Some("kind") => match words.get(2).copied() {
            None => println!("AI kind: {}", session.ai_kind.name()),
            Some(k) => match ai::AiKind::parse(k) {
                Some(kind) => set_ai_kind(session, kind),
                None => println!("usage: ai kind [heuristic|random]"),
            },
        },
        Some(s) => match parse_superpower(s) {
            Some(side) => {
                if let Some(k) = words.get(2).copied() {
                    match ai::AiKind::parse(k) {
                        Some(kind) => set_ai_kind(session, kind),
                        None => {
                            println!("usage: ai us|ussr [heuristic|random]");
                            return;
                        }
                    }
                }
                session.ai_side = Some(side);
                println!("AI ({}) now plays {side} automatically", session.ai_kind.name());
            }
            None => println!("usage: ai [us|ussr [heuristic|random]|kind [heuristic|random]|off]"),
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
    let debug = if session.debug { "[debug] " } else { "" };
    let choosing = match session.game.operation() {
        Some(Operation::Event(e)) => format!("{} choosing ", e.chooser()),
        _ => String::new(),
    };
    let round = if session.game.phase() == twilight_struggle::game::Phase::Setup { "setup".to_string() } else if status.in_headline() { "headline".to_string() } else { format!("AR {}/{}", status.action_round, status.action_rounds_per_turn) };
    format!("{debug}{} {round} {card}{choosing}> ", session.game.active())
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
                    session.ai.as_mut(),
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
            if !debug_guard(session) {
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
        "clear" => {
            if !debug_guard(session) {
                return;
            }
            match words.get(1) {
                Some(&"all") => {
                    let ids: Vec<_> = session.map.iter().map(|(id, _)| id).collect();
                    for id in ids {
                        debug_clear_country(session, id);
                    }
                    println!("cleared the whole board");
                }
                Some(_) => {
                    let query = words[1..].join(" ");
                    match session.map.find(&query) {
                        Found::One(id) => {
                            debug_clear_country(session, id);
                            println!("cleared {}", session.map.country(id).name);
                        }
                        Found::None => println!("no country matches {query:?}"),
                        Found::Ambiguous(ids) => print_ambiguous(session, &ids),
                    }
                }
                None => println!("usage: clear <country>|all"),
            }
        }
        "blank" => {
            if !debug_guard_strict(session) {
                return;
            }
            session.game = Game::from_scenario(&Scenario::blank(&session.map));
            session.game.record_note("blanked the board, status, and hands");
            println!("blanked board, status, and hands");
        }
        "vp" => {
            let Some(n) = words.get(1).and_then(|s| s.parse::<i8>().ok()) else {
                println!("usage: vp <n> (-20..=20)");
                return;
            };
            if !debug_guard_strict(session) {
                return;
            }
            let Some(before) = debug_apply_status(session, |s| s.vp = n) else { return };
            session.game.record_note(format!("debug: vp {} -> {n}", before.vp));
            println!("vp set to {n}");
        }
        "defcon" => {
            let Some(n) = words.get(1).and_then(|s| s.parse::<u8>().ok()) else {
                println!("usage: defcon <n> ({}..={})", DEFCON_RANGE.start(), DEFCON_RANGE.end());
                return;
            };
            if !debug_guard_strict(session) {
                return;
            }
            let Some(before) = debug_apply_status(session, |s| s.defcon = n) else { return };
            session.game.record_note(format!("debug: defcon {} -> {n}", before.defcon));
            println!("defcon set to {n}");
        }
        "turn" => {
            let Some(n) = words.get(1).and_then(|s| s.parse::<u8>().ok()) else {
                println!("usage: turn <n> ({}..={})", TURN_RANGE.start(), TURN_RANGE.end());
                return;
            };
            if !debug_guard_strict(session) {
                return;
            }
            let Some(before) = debug_apply_status(session, |s| s.turn = n) else { return };
            session.game.record_note(format!("debug: turn {} -> {n}", before.turn));
            println!("turn set to {n}");
        }
        "ar" => {
            let Some(n) = words.get(1).and_then(|s| s.parse::<u8>().ok()) else {
                println!("usage: ar <n> (1..=this turn's own action_rounds_per_turn)");
                return;
            };
            if !debug_guard_strict(session) {
                return;
            }
            let Some(before) = debug_apply_status(session, |s| s.action_round = n) else { return };
            session.game.record_note(format!("debug: action round {} -> {n}", before.action_round));
            println!("action round set to {n}");
        }
        "active" => {
            let Some(side) = words.get(1).and_then(|s| parse_superpower(s)) else {
                println!("usage: active us|ussr");
                return;
            };
            if !debug_guard_strict(session) {
                return;
            }
            let Some(before) = debug_apply_status(session, |s| s.active = side) else { return };
            session.game.record_note(format!("debug: active side {} -> {side}", before.active));
            println!("{side} is now active");
        }
        "china" => {
            let Some(side) = words.get(1).and_then(|s| parse_superpower(s)) else {
                println!("usage: china us|ussr [up|down]");
                return;
            };
            let face_up = match words.get(2) {
                Some(&"up") | None => true,
                Some(&"down") => false,
                Some(other) => {
                    println!("expected 'up' or 'down', got {other:?}");
                    return;
                }
            };
            if !debug_guard_strict(session) {
                return;
            }
            let Some(_before) = debug_apply_status(session, |s| {
                s.china_card = side;
                s.china_card_face_up = face_up;
            }) else {
                return;
            };
            session.game.record_note(format!("debug: china card -> {side} ({})", if face_up { "up" } else { "down" }));
            println!("China Card now held by {side}, face {}", if face_up { "up" } else { "down" });
        }
        "give" => {
            if words.len() < 3 {
                println!("usage: give us|ussr <card>");
                return;
            }
            let Some(side) = parse_superpower(words[1]) else {
                println!("expected 'us' or 'ussr', got {:?}", words[1]);
                return;
            };
            let query = words[2..].join(" ");
            let Some(id) = find_card_or_report(session, &query) else { return };
            if id == CHINA_CARD {
                println!("the China Card isn't tracked in a hand — use `china us|ussr` instead");
                return;
            }
            if !debug_guard_strict(session) {
                return;
            }
            // A card already in `side`'s hand doesn't grow it — `take`
            // removes it from there before `push_to_hand` puts it right
            // back — so only a genuine addition needs the room check.
            if !session.game.hand(side).contains(&id) && session.game.hand(side).len() >= MAX_HAND_SIZE {
                println!("{side}'s hand already has {MAX_HAND_SIZE} cards — discard or exile one first");
                return;
            }
            session.game.hands_mut().take(id);
            session.game.hands_mut().push_to_hand(side, id);
            session.game.record_note(format!("debug: gave {} to {side}", session.cards.card(id).name));
            println!("{} is now in {side}'s hand", session.cards.card(id).name);
        }
        "discard" => {
            let Some(query) = words.get(1..).map(|w| w.join(" ")).filter(|q| !q.is_empty()) else {
                println!("usage: discard <card>");
                return;
            };
            let Some(id) = find_card_or_report(session, &query) else { return };
            // Eagle/Bear has Landed: the perk's holder answering at the end of the turn.
            if session.game.awaiting_discard().is_some() {
                match session.game.discard_held(Some(id)) {
                    Ok(()) => println!("{} is discarded (Eagle/Bear has Landed)", session.cards.card(id).name),
                    Err(e) => println!("{e}"),
                }
                return;
            }
            if id == CHINA_CARD {
                println!("the China Card isn't tracked in a hand — use `china us|ussr` instead");
                return;
            }
            if !debug_guard_strict(session) {
                return;
            }
            session.game.hands_mut().take(id);
            session.game.hands_mut().discard(id);
            session.game.record_note(format!("debug: discarded {}", session.cards.card(id).name));
            println!("{} is now in the discard pile", session.cards.card(id).name);
        }
        "exile" => {
            let Some(query) = words.get(1..).map(|w| w.join(" ")).filter(|q| !q.is_empty()) else {
                println!("usage: exile <card>");
                return;
            };
            let Some(id) = find_card_or_report(session, &query) else { return };
            if id == CHINA_CARD {
                println!("the China Card isn't tracked in a hand — use `china us|ussr` instead");
                return;
            }
            if !debug_guard_strict(session) {
                return;
            }
            session.game.hands_mut().take(id);
            session.game.hands_mut().remove_from_game(id);
            session.game.record_note(format!("debug: removed {} from the game", session.cards.card(id).name));
            println!("{} is now removed from the game", session.cards.card(id).name);
        }
        "debug" => match words.get(1) {
            Some(&"on") => {
                session.debug = true;
                println!("debug mode on");
            }
            Some(&"off") => {
                session.debug = false;
                println!("debug mode off");
            }
            None => println!("debug mode is {}", if session.debug { "on" } else { "off" }),
            Some(other) => println!("usage: debug [on|off], got {other:?}"),
        },
        "states" => match session.states.list() {
            Ok(entries) => {
                let filter = words.get(1);
                let mut shown = 0;
                for entry in &entries {
                    if filter.is_some_and(|&f| f != entry.file) {
                        continue;
                    }
                    println!("{:<40} {}", entry.reference(), entry.description);
                    shown += 1;
                }
                if shown == 0 {
                    println!("no states found{}", filter.map(|f| format!(" in {f:?}")).unwrap_or_default());
                }
            }
            Err(e) => println!("{e}"),
        },
        "save" => run_save_command(session, &words),
        "new" => {
            if let Some(op) = session.game.operation() {
                println!("finish or cancel the {} first ({} of {} ops left)", op.verb(), op.remaining(), op.ops_total());
                return;
            }
            session.game = Game::new_game(&session.map, &session.cards, &mut session.dice);
            session.debug = false;
            session.game.record_note("new game");
            println!("new game: the USSR places 6 influence in Eastern Europe, then the US 7 in Western Europe");
        }
        "load" => match words.get(1) {
            Some(&"demo") => {
                let scenario = Scenario::demo(&session.map, &session.cards).expect("demo scenario should be valid");
                session.game = Game::from_scenario(&scenario);
                session.game.record_note("loaded the demo scenario");
                println!("loaded demo scenario");
            }
            Some(reference) => {
                if let Some(op) = session.game.operation() {
                    println!("finish or cancel the {} first ({} of {} ops left)", op.verb(), op.remaining(), op.ops_total());
                    return;
                }
                run_load_state_command(session, reference);
            }
            None => println!("usage: load demo | <file>/<name>"),
        },
        "play" => run_play_command(session, &words),
        "influence" => run_begin_command(session, OperationKind::Influence, &words),
        "realign" => run_begin_command(session, OperationKind::Realign, &words),
        "coup" => run_begin_command(session, OperationKind::Coup, &words),
        "event" => run_event_command(session),
        "space" => run_space_command(session),
        "spacerace" => println!("{}", render_space_track(session.game.status()).render(session.color)),
        "tracks" => {
            let tab = words.get(1).map(|w| w.to_lowercase());
            let found = match &tab {
                None => Some(TrackTab::Space),
                Some(w) => TrackTab::ALL.iter().copied().find(|t| t.label().to_lowercase().starts_with(w.as_str())),
            };
            match found {
                Some(tab) => println!("{}", render_tracks(session.game.status(), tab, "").render(session.color)),
                None => println!("tracks [space|military|defcon|vp|turn]"),
            }
        }
        "milops" => println!("{}", render_military_track(session.game.status(), "").render(session.color)),
        "track" => {
            let side = match words.get(1).copied() {
                Some("us") => Some(Superpower::Us),
                Some("ussr") => Some(Superpower::Ussr),
                _ => None,
            };
            let (Some(side), Some(n)) = (side, words.get(2).and_then(|s| s.parse::<u8>().ok())) else {
                println!("usage: track us|ussr <box> (0..=8)");
                return;
            };
            if !debug_guard_strict(session) {
                return;
            }
            let Some(before) = debug_apply_status(session, |s| twilight_struggle::space::set_position(s, side, n)) else { return };
            session.game.record_note(format!("debug: {side} space race {} -> {n}", twilight_struggle::space::position(&before, side)));
            println!("{side} space race marker set to box {n}");
        }
        "place" | "+" => run_place_command(session, &words),
        "-" | "take" => run_unplace_command(session, &words),
        "mode" => run_mode_command(session, &words),
        "roll" => run_roll_command(session, &words),
        "undo" => run_undo_command(session),
        "confirm" => run_confirm_command(session),
        "cancel" => run_cancel_command(session),
        "abandon" => run_abandon_command(session),
        "status" => run_status_command(session),
        "pass" => run_pass_command(session),
        "headline" => run_headline_command(session, &words),
        "escape" => run_escape_command(session, &words),
        "defuse" => run_defuse_command(session, &words),
        "ai" => run_ai_command(session, &words),
        "hand" => run_hand_command(session, &words),
        "piles" => run_piles_command(session, &words),
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

/// Resolves `query` via [`CardCatalog::find`], printing the usual "no
/// card matches"/"ambiguous" message and returning `None` if it didn't
/// resolve to exactly one card — the shared lookup `give`/`discard`/
/// `exile` need, so each only has to handle the one-match case.
fn find_card_or_report(session: &Session, query: &str) -> Option<CardId> {
    match session.cards.find(query) {
        CardFound::One(id) => Some(id),
        CardFound::None => {
            println!("no card matches {query:?}");
            None
        }
        CardFound::Ambiguous(ids) => {
            let names: Vec<&str> = ids.iter().map(|&id| session.cards.card(id).name.as_str()).collect();
            println!("ambiguous: {}", names.join(", "));
            None
        }
    }
}

/// Refuses `set`/`add`/`remove`/`clear` unless debug mode is on and no
/// operation is open — the same guard those three already had before
/// debug mode existed, just with debug added to it. Doesn't also check
/// for a card in play: these four only ever touch the board, which a
/// card mid-play has no stake in.
fn debug_guard(session: &Session) -> bool {
    if !session.debug {
        println!("debug mode is off — `debug on` first");
        return false;
    }
    if let Some(op) = session.game.operation() {
        println!("finish or cancel the {} first ({} of {} ops left)", op.verb(), op.remaining(), op.ops_total());
        return false;
    }
    true
}

/// [`debug_guard`], plus refusing while a card is in play — for every
/// debug edit that touches the status or the hands/piles themselves
/// (`vp`/`defcon`/`turn`/`ar`/`active`/`china`/`give`/`discard`/`exile`/
/// `blank`), where a card mid-play could otherwise end up pointing at a
/// hand that's just been rewritten out from under it.
fn debug_guard_strict(session: &Session) -> bool {
    if !debug_guard(session) {
        return false;
    }
    if let Some(card) = session.game.card_in_play() {
        println!("card #{card} is in play — play an operation with it, or return it, first");
        return false;
    }
    true
}

/// Applies `mutate` to a *copy* of the live status, validates the result
/// ([`GameStatus::validate`]) before committing anything, and returns the
/// status as it was before the change on success — so `vp`/`defcon`/
/// `turn`/`ar`/`active`/`china` all go through the same one range check
/// (rather than each duplicating it) and report what actually changed.
/// On failure, prints the validation error and leaves the real status
/// untouched; the caller's own `return` on `None` is what makes this a
/// no-op from there.
fn debug_apply_status(session: &mut Session, mutate: impl FnOnce(&mut GameStatus)) -> Option<GameStatus> {
    let before = *session.game.status();
    let mut status = before;
    mutate(&mut status);
    if let Err(e) = status.validate() {
        println!("{e}");
        return None;
    }
    *session.game.status_mut() = status;
    Some(before)
}

/// Zeroes both sides' influence in `id`, logging a [`crate::Event::Edit`]
/// for each side that actually changed — the shared body of `clear
/// <country>` and `clear all`.
fn debug_clear_country(session: &mut Session, id: twilight_struggle::CountryId) {
    for side in [Superpower::Us, Superpower::Ussr] {
        let before = session.game.board().influence(id, side);
        if before != 0 {
            session.game.board_mut().set_influence(id, side, 0);
            session.game.record_edit(id, side, before, 0);
        }
    }
}

/// `save <file>/<name> [description...]`: snapshots the live game
/// ([`Game::snapshot`]) and writes it to the named test state, creating
/// the file if needed. Refused with an operation open or a card in play
/// — a [`Scenario`] has no room for either, so saving over one would
/// silently drop it.
fn run_save_command(session: &mut Session, words: &[&str]) {
    if let Some(op) = session.game.operation() {
        println!("finish or cancel the {} first ({} of {} ops left)", op.verb(), op.remaining(), op.ops_total());
        return;
    }
    if let Some(card) = session.game.card_in_play() {
        println!("card #{card} is in play — a saved state has no room for an in-progress operation; return it first");
        return;
    }
    let Some(&reference) = words.get(1) else {
        println!("usage: save <file>/<name> [description...]");
        return;
    };
    let description = words.get(2..).map(|w| w.join(" ")).unwrap_or_default();
    let snapshot = session.game.snapshot();
    match session.states.save(&session.map, &session.cards, reference, &description, &snapshot) {
        Ok(()) => println!("saved {reference}"),
        Err(e) => println!("{e}"),
    }
}

/// `load <file>/<name>`: the shared body behind the REPL's `load`
/// command and `--state` at launch. Turns debug mode on automatically —
/// unlike `load demo`, a named test state exists specifically to be
/// edited and re-examined.
fn run_load_state_command(session: &mut Session, reference: &str) {
    match session.states.load(&session.map, &session.cards, reference) {
        Ok((scenario, description)) => {
            session.game = Game::from_scenario(&scenario);
            session.debug = true;
            session.game.record_note(format!("loaded test state {reference}"));
            if description.is_empty() {
                println!("loaded {reference} (debug mode on)");
            } else {
                println!("loaded {reference} (debug mode on): {description}");
            }
        }
        Err(e) => println!("{e}"),
    }
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
/// `space`: spends the card in play on a space race attempt and shows the result.
fn run_space_command(session: &mut Session) {
    match session.game.space(&mut session.dice) {
        Ok(result) => {
            let canvas = render_space_result(&session.cards, &result, session.game.status().vp, session.game.winner(), None);
            println!("{}", canvas.render(session.color));
        }
        Err(e) => println!("{e}"),
    }
}

fn run_event_command(session: &mut Session) {
    match session.game.play_event_with(&session.map, &session.cards, &mut session.dice) {
        Ok(EventOutcome::Scoring(result)) => {
            let vp_after = session.game.status().vp;
            let canvas = render_scoring_result(&session.map, &session.cards, &result, vp_after, None);
            println!("{}", canvas.render(session.color));
            if let Some(victory) = session.game.winner() {
                println!("{}", game_over_line(victory));
            }
        }
        Ok(EventOutcome::Effect(result)) => {
            let vp_after = session.game.status().vp;
            let winner = session.game.winner();
            let canvas = render_event_result(&session.map, &session.cards, &result, vp_after, winner, None);
            println!("{}", canvas.render(session.color));
            if session.game.ops_after_event().is_some() {
                print_turn_state(session);
            }
        }
        Ok(EventOutcome::Pending { .. }) => print_event_prompt(session),
        Err(e) => println!("{e}"),
    }
}

/// A resolved war's box, and the game-over line if it ended the game.
fn print_war_result(session: &Session, result: &twilight_struggle::events::WarResult) {
    let vp_after = session.game.status().vp;
    let canvas = render_war_result(&session.map, &session.cards, result, vp_after, session.game.winner(), None);
    println!("{}", canvas.render(session.color));
}

/// Tells whoever has to choose what the open event asks, and where.
fn print_event_prompt(session: &Session) {
    if let Some(Operation::War(w)) = session.game.operation() {
        let card = session.cards.card(w.card()).name.as_str();
        let targets: Vec<&str> = twilight_struggle::events::war::eligible_targets(&session.map, w.card())
            .into_iter()
            .map(|id| session.map.country(id).name.as_str())
            .collect();
        println!("{card} — {} declares war; needs {}+ after modifiers", w.side(), w.success_min());
        println!("targets: {}", targets.join(", "));
        println!("use roll <country> to attack (abandon backs out)");
        return;
    }
    let Some(Operation::Event(e)) = session.game.operation() else { return };
    let card = session.game.card_in_play().map(|id| session.cards.card(id).name.as_str()).unwrap_or(e.title().unwrap_or(if e.is_triggered() { "NORAD" } else { "?" }));
    println!("{card} — {} chooses: {}", e.chooser(), e.prompt());
    if e.needs_roll() {
        println!("throw the dice with: roll");
        return;
    }
    if e.is_multi() {
        for (i, &card) in e.pile().iter().enumerate() {
            println!("  {}) [{}] {}", i + 1, if e.is_marked(i) { "x" } else { " " }, session.cards.card(card).name);
        }
        println!("mark or unmark a card with: mode <n> — {} marked; confirm to finish", e.marked_count());
        return;
    }
    if e.mode().is_none() {
        println!("pick a {} with: mode <n|name>", if e.is_designation() { "region" } else { "mode" });
        return;
    }
    if e.is_designation() {
        println!("change it with: mode <n|name> — confirm to finish");
        return;
    }
    let steps = e.forward_steps(&session.map);
    let list: Vec<String> = steps
        .iter()
        .map(|&(id, sign)| {
            let n = e.can_forward(&session.map, id, sign).unwrap_or(0);
            format!("{} {}{n}", session.map.country(id).name, if sign == twilight_struggle::events::choice::Sign::Plus { '+' } else { '-' })
        })
        .collect();
    println!("available: {}", if list.is_empty() { "nothing — confirm to finish".to_string() } else { list.join(", ") });
    println!("use + <country> / - <country> (u undoes the last pick, confirm finishes) — {}", e.progress());
}

/// The last `EventResolved` log entry, printed the way `event` prints a
/// fixed-effect card's result — what a confirmed choice event produced.
fn print_last_event_result(session: &Session) {
    if let Some(entry) = session.game.log().entries().iter().rev().find(|e| matches!(e.event, twilight_struggle::Event::EventResolved { .. }))
        && let twilight_struggle::Event::EventResolved { result, vp_after } = &entry.event
    {
        let canvas = render_event_result(&session.map, &session.cards, result, *vp_after, session.game.winner(), None);
        println!("{}", canvas.render(session.color));
    }
}

/// Parses `<country words> [n]` for `+`/`-`/`place`/`take`.
fn parse_country_and_count(session: &Session, rest: &[&str]) -> Option<(twilight_struggle::CountryId, u8)> {
    let (country_words, amount) = match rest.split_last() {
        Some((&last, init)) if !init.is_empty() && last.parse::<u8>().is_ok() => (init, last.parse::<u8>().unwrap()),
        _ => (rest, 1),
    };
    let query = country_words.join(" ");
    match session.map.find(&query) {
        Found::One(id) => Some((id, amount)),
        Found::None => {
            println!("no country matches {query:?}");
            None
        }
        Found::Ambiguous(ids) => {
            print_ambiguous(session, &ids);
            None
        }
    }
}

/// `-`/`take <country> [n]`: while placing influence, takes back a pending
/// point in that country; while resolving an event, removes influence
/// there if the event allows it (or takes back a staged add).
fn run_unplace_command(session: &mut Session, words: &[&str]) {
    if session.game.operation().is_none() {
        println!("nothing open — start a placement (influence) or an event first");
        return;
    }
    let rest = &words[1..];
    if rest.is_empty() {
        println!("usage: - <country> [n]");
        return;
    }
    let Some((id, amount)) = parse_country_and_count(session, rest) else { return };
    let name = session.map.country(id).name.clone();
    for _ in 0..amount {
        if let Err(e) = session.game.unplace(&session.map, id) {
            println!("{e}");
            return;
        }
    }
    println!("{name} {}", operation_touched_summary(session, id));
    print_operation_banner(session);
}

fn operation_touched_summary(session: &Session, id: twilight_struggle::CountryId) -> String {
    match session.game.operation() {
        Some(op) => {
            let (us, ussr) = (op.delta(session.game.board(), id, Superpower::Us), op.delta(session.game.board(), id, Superpower::Ussr));
            format!("(US {us:+}, USSR {ussr:+} pending)")
        }
        None => String::new(),
    }
}

/// `mode <n|name>`: chooses which way to play an open multi-mode event
/// (1-based, or by the start of its label).
fn run_mode_command(session: &mut Session, words: &[&str]) {
    let query = words[1..].join(" ").to_lowercase();
    // A number, or a mode's own label (`mode europe` for a region pick).
    let by_label = match session.game.operation() {
        Some(Operation::Event(e)) if !query.is_empty() => e.modes().iter().position(|m| m.label.to_lowercase().starts_with(&query)),
        _ => None,
    };
    let Some(n) = query.parse::<usize>().ok().filter(|&n| n >= 1).or(by_label.map(|i| i + 1)) else {
        println!("usage: mode <n|name>   (1-based)");
        return;
    };
    match session.game.choose_mode(&session.map, n - 1) {
        Ok(()) => print_event_prompt(session),
        Err(e) => println!("{e}"),
    }
}

/// `place <country> [n]` places `n` (default 1) points of influence, one
/// at a time, stopping and reporting on the first one that's refused.
fn run_place_command(session: &mut Session, words: &[&str]) {
    match session.game.operation() {
        Some(Operation::Influence(_)) => {}
        Some(Operation::Event(_)) => {
            let rest = &words[1..];
            if rest.is_empty() {
                println!("usage: + <country> [n]");
                return;
            }
            let Some((id, amount)) = parse_country_and_count(session, rest) else { return };
            for _ in 0..amount {
                if let Err(e) = session.game.place(&session.map, id) {
                    println!("{e}");
                    return;
                }
            }
            println!("{} {}", session.map.country(id).name, operation_touched_summary(session, id));
            print_operation_banner(session);
            return;
        }
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
        // An open event waiting for its roll-off (Summit).
        match session.game.roll_contest(&session.map, &mut session.dice) {
            Ok(contest) => {
                println!("rolled: {}", twilight_struggle::choice::describe_contest(&contest));
                print_event_prompt(session);
            }
            Err(twilight_struggle::GameError::NoOperation) | Err(twilight_struggle::GameError::WrongKind { .. }) => println!("usage: roll <country>"),
            Err(e) => println!("{e}"),
        }
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
        Ok(RollOutcome::War(result)) => print_war_result(session, &result),
        Ok(RollOutcome::Realign(result)) => println!("{}", roll_result_line(&session.map, side, &result)),
        Ok(RollOutcome::Coup(result)) => {
            println!("{}", coup_result_line(&session.map, side, &result));
            // Whatever the coup set off (DEFCON, Yuri and Samantha's VP).
            if let Some(aftermath) = session.game.last_coup_aftermath() {
                let entry = twilight_struggle::LogEntry {
                    turn: session.game.status().turn,
                    action_round: session.game.status().action_round,
                    side: Some(side),
                    event: twilight_struggle::Event::CoupAftermath(aftermath),
                };
                println!("{}", log_entry_line(&session.map, &session.cards, &entry));
            }
            if let Some(victory) = session.game.winner() {
                println!("{}", game_over_line(victory));
            }
        }
        Err(GameError::Realign(e)) => println!("cannot roll in {}: {e}", session.map.country(id).name),
        Err(GameError::Coup(e)) => println!("cannot coup {}: {e}", session.map.country(id).name),
        Err(GameError::War(e)) => println!("{e}"),
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

/// `piles [discard|removed|deck]` lists the discard pile, the removed-from-play pile and the
/// draw deck's size (the deck's contents are hidden) — or just the one named.
fn run_piles_command(session: &Session, words: &[&str]) {
    let tabs: Vec<PileTab> = match words.get(1).map(|w| w.to_lowercase()) {
        None => PileTab::ALL.to_vec(),
        Some(w) => match PileTab::ALL.iter().find(|t| t.label().to_lowercase().starts_with(&w)) {
            Some(&tab) => vec![tab],
            None => {
                println!("usage: piles [discard|removed|deck]");
                return;
            }
        },
    };
    for tab in tabs {
        println!("{}", piles_text(&session.cards, session.game.hands(), tab));
    }
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
        Ok(id) if matches!(session.game.operation(), Some(Operation::Event(_))) => {
            println!("undid the last pick in {}", session.map.country(id).name);
            print_operation_banner(session);
        }
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
        Ok(Operation::Event(_)) if matches!(session.game.operation(), Some(Operation::Event(_))) => {
            // A declined discard-or-suffer card hands on to a second decision.
            print_event_prompt(session);
            return;
        }
        Ok(Operation::Event(_)) => print_last_event_result(session),
        Ok(Operation::War(_)) => unreachable!("Game::confirm refuses a war"),
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
    print_turn_state(session);
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
        Ok(Operation::Event(_)) => unreachable!("Game::cancel refuses an event"),
        Ok(Operation::War(_)) => unreachable!("Game::cancel refuses a war"),
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
    print_turn_state(session);
}

/// What comes next after an operation or event closes: the operation the
/// card's event still allows, if any, else whose turn it is.
fn print_turn_state(session: &Session) {
    match session.game.ops_after_event() {
        Some(grant) => println!(
            "{} may now conduct {} with this card's ops — or pass to skip them",
            session.game.active(),
            grant.describe()
        ),
        None => println!("{} to act", session.game.active()),
    }
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
    if session.game.clear_event_mode(&session.map) {
        println!("mode cleared — choose again with `mode`, or `abandon` again to back out of the event");
    } else if session.game.operation().is_some() {
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
    let extra = status.action_round.saturating_sub(status.action_rounds_per_turn);
    let ar = if extra > 0 {
        format!("{}/{}+{extra}", status.action_round, status.action_rounds_per_turn)
    } else {
        format!("{}/{}", status.action_round, status.action_rounds_per_turn)
    };
    let round = if session.game.phase() == twilight_struggle::game::Phase::Setup { "Setup".to_string() } else if status.in_headline() { "Headline".to_string() } else { format!("AR {ar}") };
    println!("TURN {}   {round}   {} to act   {card}   {} ops available", status.turn, session.game.active(), session.game.ops_available());
    for effect in status.lasting.active() {
        println!("in effect: {}", twilight_struggle::render::lasting_effect_line(&effect));
    }
    for effect in status.effects.active() {
        println!("in effect this turn: {}", ongoing_effect_line(&effect));
    }
    print_operation_banner(session);
    if let Some(victory) = session.game.winner() {
        println!("{}", game_over_line(victory));
    }
}

/// `headline <card>` chooses the active side's headline card for the turn (rule 4.4). The
/// first choice stays hidden; once both are in, the cards are revealed and played — each as
/// its event only — by the settle step that runs after every command.
fn run_headline_command(session: &mut Session, words: &[&str]) {
    let Some(query) = words.get(1..).map(|w| w.join(" ")).filter(|q| !q.is_empty()) else {
        println!("usage: headline <card>");
        return;
    };
    let Some(id) = find_card_or_report(session, &query) else { return };
    let side = session.game.active();
    let before = session.game.log().len();
    match session.game.headline(&session.cards, id) {
        Ok(()) => {
            for entry in &session.game.log().entries()[before..] {
                println!("{}", log_entry_line(&session.map, &session.cards, entry));
            }
            if session.game.log().len() == before {
                println!("{side} has chosen a headline card — {} to choose", session.game.active());
            }
        }
        Err(e) => println!("{e}"),
    }
}

/// `pass` forfeits the active side's turn without opening an operation.
/// Refused while one is already open — cancel it first.
/// `escape <card>`: a trapped side's (Bear Trap, Quagmire) whole action round — discard an
/// Operations card worth 2+ and roll 1-4.
fn run_escape_command(session: &mut Session, words: &[&str]) {
    if session.game.trap().is_none() {
        println!("no trap is holding this action round");
        return;
    }
    let query = words[1..].join(" ");
    let id = match session.cards.find(&query) {
        CardFound::One(id) => id,
        CardFound::None => {
            println!("usage: escape <card to discard> (an Operations card worth 2+)");
            return;
        }
        CardFound::Ambiguous(ids) => {
            let names: Vec<&str> = ids.iter().map(|&id| session.cards.card(id).name.as_str()).collect();
            println!("ambiguous: {}", names.join(", "));
            return;
        }
    };
    match session.game.escape_trap(&mut session.dice, id) {
        Ok(r) => println!("{}", twilight_struggle::render::trap_result_line(&session.cards, &r)),
        Err(e) => println!("{e}"),
    }
}

/// `defuse <country>`: Cuban Missile Crisis's way out.
fn run_defuse_command(session: &mut Session, words: &[&str]) {
    let query = words[1..].join(" ");
    let id = match session.map.find(&query) {
        Found::One(id) => id,
        Found::None => {
            println!("usage: defuse <country> (Cuba for the USSR; West Germany or Turkey for the US)");
            return;
        }
        Found::Ambiguous(ids) => {
            print_ambiguous(session, &ids);
            return;
        }
    };
    match session.game.defuse_crisis(&session.map, id) {
        Ok(()) => println!("Cuban Missile Crisis defused"),
        Err(e) => println!("{e}"),
    }
}

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

/// Kept as a `const` string, rather than inline in [`print_help`], so
/// `tests::every_command_is_mentioned_in_help` can scan it for every
/// name in [`COMMANDS`] — the "stays in step with" link that module doc
/// promises, checked rather than codegen'd (the prose here doesn't
/// reduce to a flat list the way `COMMANDS` does).
const HELP_TEXT: &str = "\
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
                          overlay, p (or space) plays it
  region <name>, 1-6      zoom into one region (europe/asia/middleeast/africa/centralamerica/southamerica)
  country <name>, /<name> a single country's detail, with all its neighbours (name or code)
  new                     start a real game: the printed starting influence, the
                          Early War cards shuffled and dealt, then the opening
                          placement (USSR 6 in Eastern Europe, US 7 in Western
                          Europe, with + and confirm) and the first headline
                          phase — or launch with `--new`
  load demo               reload the bundled demo scenario
  load <file>/<name>      load a named test state from data/states/ (Tab-
                          completes) — turns debug mode on automatically;
                          `--state <file>/<name>` loads one at launch

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
  ai us|ussr heuristic|random   ... with that kind of AI (default: heuristic)
  ai kind [heuristic|random]    show or change which AI plays (`--ai-kind` at launch)
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
  - <country> [n]         take back a pending point in that country while
                          placing influence; while resolving an event,
                          REMOVES influence there (or takes back a staged
                          add). `take` is an alias. Press - in the map too

  space                   spend the card in play on a space race attempt:
                          needs at least the next box's ops (not the China
                          Card), then a d6 at or under the box's number
                          moves the marker and pays the box's VP. The card
                          is discarded either way and the turn passes.
  spacerace               show the space race track, markers and perks
  tracks [name]           show a track: space, military, defcon, vp or turn
                          (the interactive map's t modal has them all as tabs)
  milops                  show the Military Operations track and what the
                          end of the turn will cost each side

  escape <card>           while Bear Trap (USSR) or Quagmire (US) holds your
                          action round: discard that Operations card (worth
                          2+) and roll — 1-4 ends the trap. The round is
                          spent either way. With no such card, play your
                          scoring cards, then pass.
  defuse <country>        Cuban Missile Crisis: the threatened side removes
                          2 of its own influence from Cuba (USSR) or West
                          Germany/Turkey (US), any time between operations.

  event                   play the card in play for its text. Cards with a
                          choice (Comecon, Marshall Plan, Truman Doctrine…)
                          open a session for the CARD'S OWN side to choose
                          — whoever is phasing — listing which countries
                          can take how much. Then: + <country> [n] adds
                          (also `place`), - <country> [n] removes, mode <n>
                          picks a way to play a two-mode card (Warsaw Pact,
                          South African Unrest), undo takes back the last
                          pick, and confirm finishes once the event has been
                          carried out as fully as it can be (\"may\" cards —
                          De-Stalinization, Puppet Governments — can stop
                          early). An event can't be cancelled; abandon backs
                          out before the first pick (the side that played it)
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

  headline <card>         the headline phase that opens every turn: choose
                          the active side's headline card (USSR first, US
                          second — the first choice stays hidden; a side
                          holding the Man in Earth Orbit space perk goes
                          second and sees it). Both cards are then
                          revealed and played as events, the higher
                          Operations value first (the US on a tie); the
                          US's Defectors cancels the USSR's. Or press
                          space on a card in the interactive map
  hand [us|ussr]          the named side's hand (default: active side),
                          as a strip of mini-card boxes — or see it drawn
                          under every screen inside the interactive map
  piles [discard|removed|deck]
                          the discard pile and the removed-from-play pile
                          card by card, and the draw deck's size (its
                          contents are hidden) — or press D inside the
                          interactive map (←→ tabs, z zooms a card)
  card <id|name>          one card's full detail (id, name prefix, or
                          exact name) — or press z to zoom the selected
                          card inside the interactive map

  Debug mode — direct state editing, for setting up a position to
  exercise a card against rather than playing to it. Every command below
  is refused unless `debug on` (and, except for set/add/remove/clear,
  with no card in play) — and refused, like set/add/remove already were,
  while an operation is open.
  debug [on|off]          show, or set, whether debug mode is on
  set <c> <us|ussr> <n>   set a country's influence
  add <c> <us|ussr> <n>   add influence (saturates)
  remove <c> <us|ussr> <n> remove influence (saturates at 0)
  clear <c>|all           zero a country's influence, or the whole board
  track us|ussr <box>      set a space race marker (0..8)
  vp <n>                  set the VP track (-20..20)
  defcon <n>               set DEFCON
  turn <n>, ar <n>         set the turn counter / action round
  active us|ussr           set whose turn it is to act
  china us|ussr [up|down]  set who holds the China Card, and face up/down
  give us|ussr <card>      move a card into a hand, from wherever it is
  discard <card>           move a card to the discard pile (or, at the end of
                          a turn, the Eagle/Bear has Landed holder's one
                          discard — `pass` keeps every card instead)
  exile <card>             move a card to the removed-from-game pile
  blank                    reset to an empty board, default status, and
                          empty hands — a base to build a state from
  states [file]            list named test states (data/states/), with
                          their own descriptions
  save <file>/<name> [description...]
                          save the live game as a named test state
                          (refused with a card in play — a test state has
                          no room for an in-progress operation); Tab-
                          completes the file half of an existing name

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
  quit, exit, q           leave";

fn print_help() {
    println!("{HELP_TEXT}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The link [`COMMANDS`]'s own doc promises: every command name it
    /// lists for completion is actually documented somewhere in
    /// [`HELP_TEXT`] — so a command added to one and forgotten in the
    /// other fails the build instead of just quietly drifting.
    #[test]
    fn every_command_is_mentioned_in_help() {
        for &cmd in COMMANDS {
            assert!(HELP_TEXT.contains(cmd), "{cmd:?} is in COMMANDS but not mentioned in HELP_TEXT");
        }
    }
}
