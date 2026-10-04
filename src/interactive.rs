//! Keyboard-driven navigation for the terminal — raw mode, key events, the
//! alternate screen. This is the *only* place in the binary (and the whole
//! crate) that touches the terminal directly; every view it draws still
//! comes from `twilight_struggle::render`, which stays a pure `Canvas`
//! producer per its own module doc.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

use twilight_struggle::render::{
    log_entry_line, operation_abandoned_line, pile_cards, render_piles, PileTab, operation_closed_line, operation_header, render_card, render_country, render_forced_card, render_hand,
    render_event_result, render_event_session, render_final_scoring, render_headline_confirm, render_headline_reveal, render_space_confirm, render_space_result, render_trap_confirm, render_trap_result, render_space_track_with_hint, render_war_result, render_region, render_roll_result, render_scoring_result, render_status_bar_with, render_world_map, Canvas, RollReport, HAND_ROWS,
};
use twilight_struggle::events::{PlayAs, EffectResult, ScoringResult, WarResult};
use twilight_struggle::game::{Phase, Trap, TrapResult, Victory};
use twilight_struggle::space::SpaceResult;
use twilight_struggle::ops::Operation;
use twilight_struggle::{
    ai, Board, CardCatalog, CardId, ColorMode, CountryId, Dice, Direction, Event, EventOutcome, Game, GameError, LogEntry, MapLayout,
    OperationKind, RandomAi, Region, RollOutcome, Superpower, ViewMode, WorldMap, CHINA_CARD,
};

/// How many cards the piles view can scroll through on `tab`.
fn piles_len(game: &Game, tab: PileTab) -> usize {
    pile_cards(game.hands(), tab).len()
}

/// Whichever irreversible result is currently shown as its own modal —
/// a resolved realignment roll/coup attempt, or a resolved scoring
/// card's event. Both share one queue (`modal` in [`run`]) and the same
/// Enter-to-dismiss handling, since both are "too important, and too
/// easy to miss in a dense message line, to report any other way" in
/// exactly the same sense — see [`RollReport`]'s own doc for the
/// original reasoning, which applies here unchanged.
enum Modal {
    Roll(RollReport),
    /// A scoring event's own result, plus the VP track's value right
    /// after it applied — [`twilight_struggle::events::ScoringResult`]
    /// only carries the *change*, the same split `RollReport` keeps
    /// between a roll's own result and the `before` state it needs.
    Score(ScoringResult, i8),
    /// A fixed-effect card's result, the VP track's new value, and the
    /// winner if the event just ended the game (the result alone doesn't
    /// say — a DEFCON-1 loss, say, isn't visible in it).
    Event(EffectResult, i8, Option<Victory>),
    /// A resolved war card's result, the VP track's new value, and the
    /// winner if the war ended the game.
    War(WarResult, i8, Option<Victory>),
    /// A resolved space race attempt, the VP track's new value, and the
    /// winner if it ended the game.
    Space(SpaceResult, i8, Option<Victory>),
    /// `s`: the confirmation before a space attempt rolls. Holds no data —
    /// it's drawn live from the card in play and the status, and Enter
    /// rolls only if [`Game::can_space`].
    SpaceConfirm,
    /// The confirmation before a trapped side discards this card and rolls to escape.
    TrapConfirm(CardId),
    /// A resolved escape attempt.
    Trap(TrapResult),
    /// The confirmation before a headline card is chosen.
    HeadlineConfirm(CardId),
    /// An open event that runs in a modal of its own (Summit's roll-off): drawn live from the
    /// event, keyed by `r` (roll), digits (choose), `c` (confirm and close), ⌫ (back).
    Session,
    /// `t`: the space race track, for information only — drawn live from
    /// the status, dismissed with Enter, Esc or `t` again.
    SpaceTrack,
    /// Final scoring after turn 10: each region's swing, the China Card, the VP at the end and
    /// the game's result.
    FinalScoring(Vec<(ScoringResult, i8)>, Option<Superpower>, i8, Option<Victory>),
    /// Both headline cards revealed: the USSR's, the US's, who resolves first and whether
    /// Defectors cancels the USSR's.
    Headline(Option<CardId>, Option<CardId>, Option<Superpower>, bool),
    /// `D`: the discard, removed and deck piles, for information only — drawn live from the
    /// game's hands. `zoom` shows the highlighted card in full instead of the list.
    Piles { tab: PileTab, cursor: usize, zoom: bool },
}

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
/// play again). The zoomed overlay is modal: since it obscures whichever
/// map screen is underneath, nothing that acts on the map or the open
/// operation — the arrow keys, `i`/`a`/`o`/`p`/`c`/`X`/`u`, and
/// Backspace's usual abandon/return-card cascade — reaches the board
/// while it's showing; only hand navigation (`[`/`]`) and selection
/// (`Space`, via the shared [`handle_hand_key`]) still work, `q`/Ctrl-C
/// still quit, and `Esc`, `Backspace`, and `z` all just close the overlay
/// (`Esc`/`Backspace` otherwise meaning "leave this screen"/"undo" are
/// repurposed to that one job while zoomed, rather than doing both).
/// `Space` actually playing a card closes the overlay too, the same as
/// `Esc`/`Backspace`/`z` — and, unzoomed, `c`/`X`/`p` actually handing the
/// turn over close it as well, since the newly active side's own hand
/// takes its place. This still isn't card *event* behaviour — nothing
/// here reads a card's text or triggers it, only its ops value, via
/// [`Game::play_card`].
#[allow(clippy::too_many_arguments)]
pub fn run(
    map: &WorldMap,
    layout: &MapLayout,
    cards: &CardCatalog,
    game: &mut Game,
    dice: &mut Dice,
    color: ColorMode,
    ai_side: Option<Superpower>,
    ai: &mut RandomAi,
) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut screen = Screen::World { selected: Region::Europe };
    // Remembers the last country selected in each region, so leaving a
    // region and coming back to it later re-selects the same one instead
    // of always resetting to its top-left-most country.
    let mut last_selected: HashMap<Region, CountryId> = HashMap::new();
    // The most recent refusal, shown on one extra row below the canvas
    // until the next key changes something. Cleared at the top of every
    // keypress *unless* `sticky` says to keep it — set only by the AI's
    // own end-of-turn summary, the one message worth reading through the
    // next keypress since it's not recoverable from the screen the way a
    // refusal or a confirm/cancel/pass report is (the status bar already
    // reflects those). Kept here rather than threaded into `render/`,
    // which never touches the terminal or takes free-text messages.
    let mut message: Option<String> = None;
    let mut sticky = false;
    // A resolved realignment roll or coup attempt is shown as its own
    // modal (`render::render_roll_result`) rather than a message-row line
    // — it's the one event in the whole session that's both irreversible
    // and easy to miss in a dense line of dice and modifiers. Queued
    // rather than a single `Option`, since an AI's turn can roll several
    // times (a multi-op realignment) before handing control back; each is
    // shown in turn, dismissed with Enter, the human and AI cases sharing
    // the same queue.
    let mut modal: VecDeque<Modal> = VecDeque::new();
    // Each side's selected index into its own hand (China Card appended
    // last, when it holds it) — kept per side, indexed via `side_index`,
    // so passing the turn back and forth doesn't lose either side's place.
    let mut hand_selected = HandUi::default();
    // Whether the selected card's full detail is overlaid on the current
    // screen. Survives ordinary navigation (the hand and its overlay
    // aren't tied to any one screen) but is always closed by an `Esc`
    // while it's open, and by `c`/`X`/`p` actually handing the turn over —
    // the newly active side's own hand takes its place.
    let mut zoomed = false;

    maybe_run_ai_turn(ai_side, ai, game, map, cards, dice, &mut message, &mut sticky, &mut zoomed, &mut modal);
    draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
    loop {
        match event::read()? {
            TermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                if !modal.is_empty() {
                    // The roll-result modal is strictly in front of
                    // everything else — a zoomed card can't even be open
                    // at the same time, since both the human's own `r`
                    // and the AI's turn close it the instant a roll
                    // happens. Only Enter (and Esc, as a harmless
                    // synonym) dismiss the front of the queue; nothing
                    // else reaches the map or the hand while it's up.
                    if matches!(modal.front(), Some(Modal::Session)) {
                        match key.code {
                            KeyCode::Char('r') if matches!(game.operation(), Some(Operation::Event(e)) if e.needs_roll()) => {
                                if let Err(e) = game.roll_contest(map, dice) {
                                    message = Some(e.to_string());
                                }
                            }
                            KeyCode::Up | KeyCode::Char('[') | KeyCode::BackTab => game.move_event_cursor(-1),
                            KeyCode::Down | KeyCode::Char(']') | KeyCode::Tab => game.move_event_cursor(1),
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                if let Err(e) = game.choose_event_cursor(map) {
                                    message = Some(e.to_string());
                                }
                            }
                            KeyCode::Char(d @ '1'..='9') => {
                                if let Err(e) = game.choose_mode(map, d as usize - '1' as usize) {
                                    message = Some(e.to_string());
                                }
                            }
                            KeyCode::Char('c') => {
                                let logged = game.log().entries().len();
                                match game.confirm() {
                                    Ok(op) => {
                                        // The modal already showed what it did: no result modal on top —
                                        // except for a card taken from the discard pile, which the
                                        // opponent must be shown.
                                        modal.pop_front();
                                        message = Some(operation_closed_line(&op, true, game.decider()));
                                        let entries = game.log().entries();
                                        if let Some(at) = entries[logged..].iter().rposition(|e| matches!(&e.event, Event::EventResolved { result, .. } if !result.takes.is_empty() || result.plays.is_some() || !result.discards.is_empty() || !result.pile_discards.is_empty())) {
                                            if let Event::EventResolved { result, .. } = &entries[logged + at].event {
                                                let mut names: Vec<String> = result.takes.iter().map(|&(side, c)| format!("{side} takes {} from the discard pile", cards.card(c).name)).collect();
                                                names.extend(result.plays.map(|p| format!("{} is now in play — {}", cards.card(p.id).name, match p.how { PlayAs::Event => "e to play its event", PlayAs::Either => "e event, or i/a/o", PlayAs::Ops => "i/a/o" })));
                                                if !names.is_empty() {
                                                    message = Some(names.join(" · "));
                                                }
                                            }
                                            queue_turn_modals(&mut modal, game.board(), &entries[logged + at..]);
                                        }
                                    }
                                    Err(e) => message = Some(e.to_string()),
                                }
                            }
                            KeyCode::Backspace | KeyCode::Esc => {
                                if game.clear_event_mode(map) {
                                    // Back to choosing.
                                } else if matches!(game.operation(), Some(Operation::Event(e)) if e.needs_roll() || e.is_pile_pick()) {
                                    // Not rolled / chosen yet: take the card back.
                                    match game.abandon() {
                                        Ok(op) => {
                                            modal.pop_front();
                                            message = Some(operation_abandoned_line(&op));
                                        }
                                        Err(e) => message = Some(e.to_string()),
                                    }
                                }
                            }
                            KeyCode::Char('q') => return Ok(()),
                            _ => {}
                        }
                        maybe_run_ai_turn(ai_side, ai, game, map, cards, dice, &mut message, &mut sticky, &mut zoomed, &mut modal);
                        prune_session_modal(&mut modal, game);
                        ensure_session_modal(&mut modal, game);
                        draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
                        continue;
                    }
                    if let Some(&Modal::TrapConfirm(card)) = modal.front() {
                        match key.code {
                            KeyCode::Enter | KeyCode::Char('r') => {
                                modal.pop_front();
                                match game.escape_trap(dice, card) {
                                    Ok(result) => modal.push_back(Modal::Trap(result)),
                                    Err(e) => message = Some(e.to_string()),
                                }
                            }
                            KeyCode::Esc | KeyCode::Backspace => {
                                modal.pop_front();
                            }
                            KeyCode::Char('q') => return Ok(()),
                            _ => {}
                        }
                        maybe_run_ai_turn(ai_side, ai, game, map, cards, dice, &mut message, &mut sticky, &mut zoomed, &mut modal);
                        draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
                        continue;
                    }
                    if let Some(&Modal::HeadlineConfirm(card)) = modal.front() {
                        match key.code {
                            KeyCode::Enter => {
                                modal.pop_front();
                                let side = game.active();
                                let before = game.log().len();
                                message = Some(match game.headline(cards, card) {
                                    Ok(()) => {
                                        let new_entries = &game.log().entries()[before..];
                                        queue_turn_modals(&mut modal, game.board(), new_entries);
                                        if new_entries.iter().any(|e| matches!(e.event, Event::Headline { .. })) {
                                            format!("{side} chooses {} — both headlines are in", cards.card(card).name)
                                        } else {
                                            format!("{side} has chosen a headline card — {} to choose", game.active())
                                        }
                                    }
                                    Err(e) => e.to_string(),
                                });
                            }
                            KeyCode::Esc | KeyCode::Backspace => {
                                modal.pop_front();
                            }
                            KeyCode::Char('q') => return Ok(()),
                            _ => {}
                        }
                        maybe_run_ai_turn(ai_side, ai, game, map, cards, dice, &mut message, &mut sticky, &mut zoomed, &mut modal);
                        prune_session_modal(&mut modal, game);
                        ensure_session_modal(&mut modal, game);
                        draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
                        continue;
                    }
                    if matches!(modal.front(), Some(Modal::SpaceConfirm)) {
                        // A confirmation, not a result: Enter rolls (only
                        // when allowed — otherwise it does nothing and the
                        // modal says why), Esc/Backspace cancel for free.
                        match key.code {
                            KeyCode::Enter | KeyCode::Char('r') if game.can_space() => {
                                modal.pop_front();
                                match game.space(dice) {
                                    Ok(result) => modal.push_back(Modal::Space(result, game.status().vp, game.winner())),
                                    Err(e) => message = Some(e.to_string()),
                                }
                            }
                            KeyCode::Esc | KeyCode::Backspace => {
                                modal.pop_front();
                            }
                            KeyCode::Char('q') => return Ok(()),
                            _ => {}
                        }
                        draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
                        continue;
                    }
                    if let Some(Modal::Piles { tab, cursor, zoom }) = modal.front_mut() {
                        let len = piles_len(game, *tab);
                        match key.code {
                            KeyCode::Char('q') => return Ok(()),
                            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('D') | KeyCode::Backspace if *zoom => *zoom = false,
                            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('D') | KeyCode::Backspace => {
                                modal.pop_front();
                            }
                            KeyCode::Right | KeyCode::Char(']') | KeyCode::Tab => (*tab, *cursor) = (tab.next(), 0),
                            KeyCode::Left | KeyCode::Char('[') | KeyCode::BackTab => (*tab, *cursor) = (tab.prev(), 0),
                            KeyCode::Down | KeyCode::Char('j') => *cursor = (*cursor + 1).min(len.saturating_sub(1)),
                            KeyCode::Up | KeyCode::Char('k') => *cursor = cursor.saturating_sub(1),
                            KeyCode::Char('z') if *tab != PileTab::Deck && len > 0 => *zoom = !*zoom,
                            _ => {}
                        }
                        draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
                        continue;
                    }
                    match key.code {
                        KeyCode::Enter | KeyCode::Esc => {
                            modal.pop_front();
                        }
                        KeyCode::Char('t') | KeyCode::Backspace if matches!(modal.front(), Some(Modal::SpaceTrack)) => {
                            modal.pop_front();
                        }
                        KeyCode::Char('q') => return Ok(()),
                        _ => {}
                    }
                    draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
                    continue;
                }
                if !std::mem::take(&mut sticky) {
                    message = None;
                }
                // `c` still confirms a choice that's waiting for it (a discard decision, say)
                // while a card is zoomed: the player is usually looking at the card they picked.
                let confirming = key.code == KeyCode::Char('c') && pending_choice_reminder(game, cards).is_some();
                if zoomed && !confirming {
                    // The zoomed card is modal: it obscures the map, so
                    // only hand navigation/selection and the keys that
                    // close the overlay do anything here — no operation or
                    // map-navigation key reaches the match arms below.
                    match key.code {
                        KeyCode::Esc | KeyCode::Backspace => zoomed = false,
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('z') => zoomed = false,
                        KeyCode::Char('[') | KeyCode::BackTab | KeyCode::Char(']') | KeyCode::Tab | KeyCode::Char(' ') | KeyCode::Char('p') => {
                            if let Some(m) = handle_hand_key(if key.code == KeyCode::Char('p') { KeyCode::Char(' ') } else { key.code }, game, map, cards, &mut hand_selected, &mut zoomed, &mut modal) {
                                message = Some(m);
                            }
                        }
                        _ => {}
                    }
                    draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
                    continue;
                }
                match key.code {
                    KeyCode::Esc if matches!(screen, Screen::World { .. }) => return Ok(()),
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('v') => {
                        let opponent = game.active().opponent();
                        if peeking_allowed(game) {
                            hand_selected.peek = !hand_selected.peek;
                            zoomed = false;
                        } else {
                            hand_selected.peek = false;
                            message = Some(format!("the {opponent} hand isn't revealed this turn"));
                        }
                    }
                    KeyCode::Char('z') => {
                        if zoom_card(game, &hand_selected).is_some() {
                            zoomed = !zoomed;
                        }
                    }
                    KeyCode::Char('[') | KeyCode::BackTab | KeyCode::Char(']') | KeyCode::Tab | KeyCode::Char(' ') => {
                        if let Some(m) = handle_hand_key(key.code, game, map, cards, &mut hand_selected, &mut zoomed, &mut modal) {
                            message = Some(m);
                        }
                    }
                    KeyCode::Char('i') => message = begin(game, OperationKind::Influence),
                    KeyCode::Char('a') => message = begin(game, OperationKind::Realign),
                    KeyCode::Char('o') => message = begin(game, OperationKind::Coup),
                    KeyCode::Char('t') => {
                        zoomed = false;
                        modal.push_back(Modal::SpaceTrack);
                    }
                    KeyCode::Char('D') => {
                        zoomed = false;
                        modal.push_back(Modal::Piles { tab: PileTab::Discard, cursor: 0, zoom: false });
                    }
                    // Cuban Missile Crisis: the threatened side removes 2 of its own influence from the selected country.
                    KeyCode::Char('d') => match &screen {
                        Screen::Region { selected, .. } | Screen::Country { selected, .. } => {
                            message = Some(match game.defuse_crisis(map, *selected) {
                                Ok(()) => "Cuban Missile Crisis defused".to_string(),
                                Err(e) => e.to_string(),
                            });
                        }
                        Screen::World { .. } => message = Some("select the country first (Cuba, West Germany or Turkey)".to_string()),
                    },
                    // A space-race refusal (too few ops, attempt used, …) still
                    // opens the confirmation, which explains it; only having
                    // no card in play, an open operation, or a finished game
                    // is refused outright.
                    KeyCode::Char('s') => match game.space_check() {
                        Ok(()) | Err(GameError::Space(_)) => {
                            zoomed = false;
                            modal.push_back(Modal::SpaceConfirm);
                        }
                        Err(e) => message = Some(e.to_string()),
                    },
                    KeyCode::Char('e') => match game.play_event_with(map, cards, dice) {
                        Ok(EventOutcome::Scoring(result)) => {
                            let vp_after = game.status().vp;
                            zoomed = false;
                            modal.push_back(Modal::Score(result, vp_after));
                        }
                        Ok(EventOutcome::Effect(result)) => {
                            let vp_after = game.status().vp;
                            zoomed = false;
                            modal.push_back(Modal::Event(result, vp_after, game.winner()));
                        }
                        // A choice card: the chooser's picks happen next, on
                        // the map — the status bar names who and what.
                        Ok(EventOutcome::Pending { .. }) => {
                            zoomed = false;
                            message = None;
                            ensure_session_modal(&mut modal, game);
                            // A war: a lone target (Korean War) goes straight to
                            // its country screen, ready for `r`; otherwise to a
                            // region view where only the legal targets are live.
                            if let Some(Operation::War(w)) = game.operation() {
                                let targets = twilight_struggle::events::war::eligible_targets(map, w.card());
                                let current = match &screen {
                                    Screen::Region { region, .. } | Screen::Country { region, .. } => Some(*region),
                                    Screen::World { .. } => None,
                                };
                                let first = targets.iter().copied().find(|&t| Some(map.country(t).region) == current).or(targets.first().copied());
                                if let Some(target) = first {
                                    let region = map.country(target).region;
                                    screen = if targets.len() == 1 { Screen::Country { region, selected: target } } else { Screen::Region { region, selected: target } };
                                }
                            }
                            // A region designation (Chernobyl) is made on the
                            // world map: go straight there, keeping the region
                            // we were looking at highlighted (and remembered).
                            if designating(game) {
                                match &screen {
                                    Screen::Region { region, selected } | Screen::Country { region, selected } => {
                                        last_selected.insert(*region, *selected);
                                        screen = Screen::World { selected: *region };
                                    }
                                    Screen::World { .. } => {}
                                }
                            }
                        }
                        Err(e) => message = Some(e.to_string()),
                    },
                    KeyCode::Char(d @ '1'..='9') => {
                        if let Some(Operation::Event(_)) = game.operation() {
                            match game.choose_mode(map, d as usize - '1' as usize) {
                                Err(e) => message = Some(e.to_string()),
                                // A choice with no countries to pick is settled by the mode alone: say what it does.
                                Ok(()) => {
                                    if let Some(Operation::Event(e)) = game.operation()
                                        && !e.picks_countries()
                                        && let Some(i) = e.mode()
                                    {
                                        message = Some(format!("{} — c to confirm, ⌫ to undo", e.modes()[i].label));
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char('p') if p_plays(game, &hand_selected) => {
                        if let Some(m) = handle_hand_key(KeyCode::Char(' '), game, map, cards, &mut hand_selected, &mut zoomed, &mut modal) {
                            message = Some(m);
                        }
                    }
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
                        let logged = game.log().entries().len();
                        message = Some(match game.confirm() {
                            Ok(op) => {
                                zoomed = false;
                                if let Operation::Event(_) = op {
                                    // Only what this confirm logged: a declined gate hands on to a
                                    // follow-up and resolves nothing yet.
                                    let entries = game.log().entries();
                                    if let Some(at) = entries[logged..].iter().rposition(|e| matches!(e.event, Event::EventResolved { .. })) {
                                        queue_turn_modals(&mut modal, game.board(), &entries[logged + at..]);
                                    }
                                }
                                operation_closed_line(&op, true, game.decider())
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
                        let gate = discard_gate_open(game);
                        message = Some(if game.clear_event_mode(map) {
                            if gate { "choice cleared — pick a card (space) or keep your cards (1)".to_string() } else { "mode cleared — choose again, or ⌫ to abandon the event".to_string() }
                        } else if game.operation().is_some() {
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
                            // Chernobyl-style designation: Enter picks the
                            // highlighted region instead of opening it.
                            KeyCode::Enter if designating(game) => {
                                let region = *selected;
                                let mode = Region::ALL.iter().position(|&r| r == region).expect("every region is in ALL");
                                if let Err(e) = game.choose_mode(map, mode) {
                                    message = Some(e.to_string());
                                }
                            }
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
                            KeyCode::Char('+') | KeyCode::Char('=') => match game.place(map, *selected) {
                                Err(GameError::Placement(e)) => message = Some(format!("{}: {e}", map.country(*selected).name)),
                                Err(e @ GameError::Event(_)) => message = Some(e.to_string()),
                                _ => {}
                            },
                            KeyCode::Char('-') => match game.unplace(map, *selected) {
                                Err(e @ GameError::Event(_)) => message = Some(e.to_string()),
                                Err(GameError::NothingToUndo) => {
                                    message = Some(format!("{}: nothing pending here to take back", map.country(*selected).name))
                                }
                                _ => {}
                            },
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
                            KeyCode::Char('+') | KeyCode::Char('=') => match game.place(map, *selected) {
                                Err(GameError::Placement(e)) => message = Some(format!("{}: {e}", map.country(*selected).name)),
                                Err(e @ GameError::Event(_)) => message = Some(e.to_string()),
                                _ => {}
                            },
                            KeyCode::Char('-') => match game.unplace(map, *selected) {
                                Err(e @ GameError::Event(_)) => message = Some(e.to_string()),
                                Err(GameError::NothingToUndo) => {
                                    message = Some(format!("{}: nothing pending here to take back", map.country(*selected).name))
                                }
                                _ => {}
                            },
                            KeyCode::Char('r') => {
                                let side = game.active();
                                let before = (game.board().influence(*selected, Superpower::Us), game.board().influence(*selected, Superpower::Ussr));
                                match game.roll(map, *selected, dice) {
                                    Ok(RollOutcome::War(result)) => modal.push_back(Modal::War(result, game.status().vp, game.winner())),
                                    Ok(outcome) => {
                                        let aftermath = if matches!(outcome, RollOutcome::Coup(_)) { game.last_coup_aftermath() } else { None };
                                        modal.push_back(Modal::Roll(RollReport { side, outcome, before, aftermath }))
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
                maybe_run_ai_turn(ai_side, ai, game, map, cards, dice, &mut message, &mut sticky, &mut zoomed, &mut modal);
                prune_session_modal(&mut modal, game);
                ensure_session_modal(&mut modal, game);
                draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?;
            }
            TermEvent::Resize(_, _) => draw(&screen, map, layout, cards, game, message.as_deref(), &hand_selected, zoomed, &modal, color)?,
            _ => {}
        }
    }
}

/// If `ai_side` names whoever's active right now, plays that whole turn
/// via [`ai::play_turn`] — a no-op otherwise (no AI side set, it's the
/// human's turn, or [`Game::winner`] is already set — once it is,
/// `Game::active` never changes again, so without this check every
/// later keypress would see the same AI side still "active" and try to
/// replay an already-finished turn). Turns the turn's own log entries
/// into the next message (joined by `\n`, marked `sticky` since — like
/// the modal queue below — it isn't otherwise recoverable from the
/// screen the way a refusal or a confirm/cancel/pass report already is),
/// queues a [`Modal`] for every realignment roll, coup attempt, or
/// scoring event the turn made ([`queue_turn_modals`]) — the human and
/// the AI share the same queue, dismissed the same way — and closes any
/// open zoom overlay, the same way `c`/`X`/`p` do for a human-ended turn.
#[allow(clippy::too_many_arguments)]
fn maybe_run_ai_turn(
    ai_side: Option<Superpower>,
    ai: &mut RandomAi,
    game: &mut Game,
    map: &WorldMap,
    cards: &CardCatalog,
    dice: &mut Dice,
    message: &mut Option<String>,
    sticky: &mut bool,
    zoomed: &mut bool,
    modal: &mut VecDeque<Modal>,
) {
    // What the last action round set off (NORAD) needs the map to settle.
    let before = game.log().len();
    game.settle(map, cards, dice);
    let settled: Vec<String> = game.log().entries()[before..].iter().map(|e| log_entry_line(map, cards, e)).collect();
    if !settled.is_empty() {
        queue_turn_modals(modal, game.board(), &game.log().entries()[before..]);
        *message = Some(settled.join(" · "));
        *sticky = true;
        *zoomed = false;
    }
    // `decider`, not `active`: an event's chooser is the card's own side. A
    // few rounds, since one human move can hand the AI an event to resolve
    // and then its own turn straight after.
    let mut lines: Vec<String> = Vec::new();
    let mut played = false;
    for _ in 0..3 {
        if ai_side != Some(game.decider()) || game.winner().is_some() {
            break;
        }
        played = true;
        let before = game.log().len();
        let outcome = ai::play_turn(ai, game, map, cards, dice);
        let new_entries = &game.log().entries()[before..];
        lines.extend(new_entries.iter().map(|entry| log_entry_line(map, cards, entry)));
        if let Err(e) = outcome {
            lines.push(format!("AI error: {e}"));
        }
        queue_turn_modals(modal, game.board(), new_entries);
    }
    if played {
        let side = ai_side.expect("played implies an AI side");
        *message = Some(format!("{side} (AI) plays: {}", lines.join(" · ")));
        *sticky = true;
        *zoomed = false;
    }
}

/// Whether an open event only designates a region (Chernobyl), so the
/// world map's Enter should pick the highlighted region rather than zoom
/// into it.
fn designating(game: &Game) -> bool {
    matches!(game.operation(), Some(Operation::Event(e)) if e.is_designation())
}

/// Rebuilds a [`RollReport`] for every [`Event::Realign`]/[`Event::Coup`]
/// entry in `entries` (one AI turn's worth of newly-pushed log lines),
/// each needing the target country's influence the instant *before* that
/// roll — which the log itself doesn't carry, only what changed. Walked
/// in reverse from `board` (the real board, already holding every one of
/// these rolls' effects): undoing the last roll on a given country
/// recovers the influence state right after the roll before it touched
/// that same country (or, if none did, the state the whole turn started
/// from) — exactly the "before" the earlier roll needs, and exactly the
/// "after" it leaves behind for an even earlier roll on the same country
/// to build from in turn. A realignment/coup turn's rolls are the only
/// board-mutating events in its own log slice (a placement can't share a
/// turn with either), so no other entry needs accounting for here.
fn reconstruct_roll_reports(board: &Board, entries: &[LogEntry]) -> Vec<RollReport> {
    let mut state: HashMap<CountryId, (u8, u8)> = HashMap::new();
    let mut reports = VecDeque::new();
    // Walking in reverse, a coup's aftermath entry comes just before the
    // coup entry it belongs to.
    let mut aftermath = None;
    for entry in entries.iter().rev() {
        let (side, target, outcome) = match &entry.event {
            Event::CoupAftermath(a) => {
                aftermath = Some(*a);
                continue;
            }
            Event::Realign(result) => (entry.side.expect("a realignment roll always belongs to a side"), result.target, RollOutcome::Realign(*result)),
            Event::Coup(result) => (entry.side.expect("a coup attempt always belongs to a side"), result.target, RollOutcome::Coup(*result)),
            _ => continue,
        };
        let after = *state
            .entry(target)
            .or_insert_with(|| (board.influence(target, Superpower::Us), board.influence(target, Superpower::Ussr)));
        let before = match &outcome {
            RollOutcome::Realign(result) => match result.loser {
                Some(Superpower::Us) => (after.0 + result.removed, after.1),
                Some(Superpower::Ussr) => (after.0, after.1 + result.removed),
                None => after,
            },
            RollOutcome::War(_) => unreachable!("only realignments and coups are reconstructed"),
            RollOutcome::Coup(result) => {
                // `removed` came out of the opponent's pile, `added` went
                // into the acting side's own — different fields, so both
                // undo independently onto `after` with no risk of
                // double-counting the same number twice.
                let mut before = after;
                match side.opponent() {
                    Superpower::Us => before.0 += result.removed,
                    Superpower::Ussr => before.1 += result.removed,
                }
                match side {
                    Superpower::Us => before.0 = before.0.saturating_sub(result.added),
                    Superpower::Ussr => before.1 = before.1.saturating_sub(result.added),
                }
                before
            }
        };
        state.insert(target, before);
        let aftermath = if matches!(outcome, RollOutcome::Coup(_)) { aftermath.take() } else { None };
        reports.push_front(RollReport { side, outcome, before, aftermath });
    }
    reports.into_iter().collect()
}

/// Walks one turn's new log entries in the order they actually happened,
/// queueing a [`Modal`] for each `Event::Realign`/`Event::Coup` (paired
/// off against [`reconstruct_roll_reports`]'s own output, which comes
/// back in the same relative order) and each `Event::Scored` (no
/// reconstruction needed — the entry already carries both the result
/// and the VP it landed on). A scoring card never shares a turn with a
/// roll (it has no ops to open a realignment or coup with), so there's
/// no real turn where the two interleave, but walking forward like this
/// costs nothing and stays correct if that ever changes.
fn queue_turn_modals(modal: &mut VecDeque<Modal>, board: &Board, entries: &[LogEntry]) {
    let mut rolls = reconstruct_roll_reports(board, entries).into_iter();
    for (i, entry) in entries.iter().enumerate() {
        match &entry.event {
            Event::Realign(_) | Event::Coup(_) => {
                if let Some(report) = rolls.next() {
                    modal.push_back(Modal::Roll(report));
                }
            }
            Event::Scored { result, vp_after } => {
                modal.push_back(Modal::Score(result.clone(), *vp_after));
            }
            Event::War { result, vp_after } => {
                let winner = match entries.get(i + 1).map(|e| &e.event) {
                    Some(Event::GameOver(victory)) => Some(*victory),
                    _ => None,
                };
                modal.push_back(Modal::War(result.clone(), *vp_after, winner));
            }
            Event::Headline { ussr, us, first, cancelled } => modal.push_back(Modal::Headline(*ussr, *us, *first, *cancelled)),
            Event::FinalScoring { results, china, vp_after } => {
                let winner = match entries.get(i + 1).map(|e| &e.event) {
                    Some(Event::GameOver(victory)) => Some(*victory),
                    _ => None,
                };
                modal.push_back(Modal::FinalScoring(results.clone(), *china, *vp_after, winner));
            }
            Event::Trap(result) => modal.push_back(Modal::Trap(*result)),
            Event::Space { result, vp_after } => {
                let winner = match entries.get(i + 1).map(|e| &e.event) {
                    Some(Event::GameOver(victory)) => Some(*victory),
                    _ => None,
                };
                modal.push_back(Modal::Space(*result, *vp_after, winner));
            }
            Event::EventResolved { result, vp_after } => {
                let winner = match entries.get(i + 1).map(|e| &e.event) {
                    Some(Event::GameOver(victory)) => Some(*victory),
                    _ => None,
                };
                modal.push_back(Modal::Event(result.clone(), *vp_after, winner));
            }
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

/// The hand strip's state: each side's selected slot, and whether it is
/// showing the opponent's (revealed) hand instead of the active side's own.
#[derive(Debug, Default)]
struct HandUi {
    selected: [usize; 2],
    peek: bool,
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
fn selected_hand_card(game: &Game, hand_selected: &HandUi) -> Option<CardId> {
    let side = hand_side(game, hand_selected.peek);
    let hand = game.hand(side);
    let count = hand_item_count(game, side);
    if count == 0 {
        return None;
    }
    let idx = hand_selected.selected[side_index(side)].min(count - 1);
    Some(if idx < hand.len() { hand[idx] } else { CHINA_CARD })
}

/// The card `z` zooms on: whichever card is in play, if any (it has left
/// the hand, so the strip's selection no longer points at it), else the
/// strip's current selection.
fn zoom_card(game: &Game, hand_selected: &HandUi) -> Option<CardId> {
    if discard_gate_open(game) || hand_selected.peek {
        return selected_hand_card(game, hand_selected);
    }
    game.card_in_play().or_else(|| selected_hand_card(game, hand_selected))
}

/// Opens a [`Modal::Session`] for an open event that runs in one but has none showing — one the
/// AI started, say, whose choice falls to the human.
fn ensure_session_modal(modal: &mut VecDeque<Modal>, game: &Game) {
    if matches!(game.operation(), Some(Operation::Event(e)) if e.has_session_modal()) && !modal.iter().any(|m| matches!(m, Modal::Session)) {
        modal.push_back(Modal::Session);
    }
}

/// Closes a [`Modal::Session`] whose event is no longer open (confirmed, or settled by the AI).
fn prune_session_modal(modal: &mut VecDeque<Modal>, game: &Game) {
    while matches!(modal.front(), Some(Modal::Session)) && !matches!(game.operation(), Some(Operation::Event(e)) if e.has_session_modal()) {
        modal.pop_front();
    }
}

/// What an open event that is settled by its chosen mode alone (a discard
/// decision, a DEFCON level) still needs from its player: confirm it, or
/// undo it. Shown for as long as that is true, whatever else is pressed.
fn pending_choice_reminder(game: &Game, cards: &CardCatalog) -> Option<String> {
    let Some(Operation::Event(e)) = game.operation() else { return None };
    if e.picks_countries() || e.is_designation() {
        return None;
    }
    let i = e.mode()?;
    Some(match e.chosen_discard() {
        Some(card) if e.gate_side() != e.chooser() => {
            format!("{} will discard {} from the {} hand — c to confirm, ⌫ to undo", e.chooser(), cards.card(card).name, e.gate_side())
        }
        Some(card) => format!("{} will discard {} — c to confirm, ⌫ to undo", e.chooser(), cards.card(card).name),
        None => format!("{} — c to confirm, ⌫ to undo", e.modes()[i].label),
    })
}

/// Whether an open event is a discard-or-suffer decision (Blockade, Debt
/// Crisis), whose victim picks a card from their own hand.
fn discard_gate_open(game: &Game) -> bool {
    matches!(game.operation(), Some(Operation::Event(e)) if !e.gate_cards().is_empty())
}

/// Whose hand the strip shows and navigates: the side that has to discard,
/// while a discard decision is open, else the active side.
fn hand_side(game: &Game, peek: bool) -> Superpower {
    match game.operation() {
        Some(Operation::Event(e)) if !e.gate_cards().is_empty() => e.gate_side(),
        _ if peek && peeking_allowed(game) => game.active().opponent(),
        _ => game.active(),
    }
}

/// Whether the active side may look at its opponent's hand right now: only
/// while an event (CIA Created, "Lone Gunman", Aldrich Ames Remix) has
/// revealed it for the turn.
fn peeking_allowed(game: &Game) -> bool {
    game.status().effects.hand_revealed(game.active().opponent())
}

/// Whether `p` plays the selected card (like `Space`) rather than passing:
/// nothing is in play, there is a card to select, and no held-card discard
/// is being asked for (there `p` keeps the hand). Pass is only legal when
/// the hand is empty or a card is already in play, so the two never clash.
fn p_plays(game: &Game, hand_selected: &HandUi) -> bool {
    game.card_in_play().is_none() && game.awaiting_discard().is_none() && hand_item_count(game, hand_side(game, hand_selected.peek)) > 0
}

/// `[`/`]`/`Space` share this handler between the normal keymap and the
/// zoomed-card modal (see `run`'s own doc), since both let hand
/// navigation and selection work identically. `[`/`]` cycle the
/// selection and return `None`; `Space` plays the selected card via
/// [`Game::play_card`], clearing `*zoomed` on success (so playing a card
/// closes an open zoom, same as selecting it) and returning the status
/// message either way. A no-op (returning `None`, `*zoomed` untouched) on
/// an empty hand.
fn handle_hand_key(code: KeyCode, game: &mut Game, map: &WorldMap, cards: &CardCatalog, hand_selected: &mut HandUi, zoomed: &mut bool, modal: &mut VecDeque<Modal>) -> Option<String> {
    match code {
        KeyCode::Char('[') | KeyCode::BackTab => {
            cycle_hand(game, hand_selected, -1);
            None
        }
        KeyCode::Char(']') | KeyCode::Tab => {
            cycle_hand(game, hand_selected, 1);
            None
        }
        KeyCode::Char(' ') if discard_gate_open(game) => {
            // Choose to discard the highlighted card (confirm with `c`).
            let id = selected_hand_card(game, hand_selected)?;
            let name = &cards.card(id).name;
            let Some(Operation::Event(e)) = game.operation() else { return None };
            let Some(slot) = e.gate_cards().iter().position(|&c| c == id) else {
                return Some(format!("{name} can't be chosen for this — pick another card"));
            };
            let side = e.chooser();
            let e_verb = e.gate_verb();
            let mode = slot + e.gate_offset();
            Some(match game.choose_mode(map, mode) {
                Ok(()) => format!("{side} will {} {name} — c to confirm", e_verb),
                Err(e) => e.to_string(),
            })
        }
        KeyCode::Char(' ') if hand_selected.peek && peeking_allowed(game) => {
            Some(format!("that's the {} hand — v to go back to your own", game.active().opponent()))
        }
        KeyCode::Char(' ') if matches!(game.trap(), Some((_, Trap::Escape(_)))) => {
            // A trapped action round: the selected card is the one to discard, after a confirmation.
            let id = selected_hand_card(game, hand_selected)?;
            let name = &cards.card(id).name;
            let Some((_, Trap::Escape(candidates))) = game.trap() else { return None };
            if !candidates.contains(&id) {
                return Some(format!("{name} can't be discarded to escape — pick an Operations card worth 2 or more"));
            }
            *zoomed = false;
            modal.push_back(Modal::TrapConfirm(id));
            None
        }
        KeyCode::Char(' ') if game.phase() == Phase::Headline && !hand_selected.peek => {
            let id = selected_hand_card(game, hand_selected)?;
            *zoomed = false;
            modal.push_back(Modal::HeadlineConfirm(id));
            None
        }
        KeyCode::Char(' ') if game.awaiting_discard().is_some() => {
            let id = selected_hand_card(game, hand_selected)?;
            let side = game.active();
            Some(match game.discard_held(Some(id)) {
                Ok(()) => format!("{side} discards {} (Eagle/Bear has Landed)", cards.card(id).name),
                Err(e) => e.to_string(),
            })
        }
        KeyCode::Char(' ') => {
            let id = selected_hand_card(game, hand_selected)?;
            let side = game.active();
            Some(match game.play_card(cards, id) {
                Ok(()) => {
                    *zoomed = false;
                    format!("{side} plays {} ({} ops) — i/a/o to use them", cards.card(id).name, cards.card(id).ops)
                }
                Err(e) => e.to_string(),
            })
        }
        _ => None,
    }
}

/// `[`/`]`: moves the active side's own hand selection by `delta` (`-1` or
/// `1`), wrapping around either end. A no-op when that side's hand (plus a
/// possible China Card) is empty — there's nothing to select.
fn cycle_hand(game: &Game, hand_selected: &mut HandUi, delta: i32) {
    let side = hand_side(game, hand_selected.peek);
    let count = hand_item_count(game, side);
    if count == 0 || (game.card_in_play().is_some() && !discard_gate_open(game) && !hand_selected.peek) {
        return;
    }
    let idx = side_index(side);
    let current = hand_selected.selected[idx].min(count - 1) as i32;
    hand_selected.selected[idx] = (current + delta).rem_euclid(count as i32) as usize;
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

/// Blits `overlay` centred on `canvas`, widening `canvas` first (its
/// existing content pinned at the top-left) if `overlay` is bigger in
/// either dimension — shared by the zoomed-card overlay and the
/// roll-result modal below, neither of which should ever be clipped by a
/// view too small for it.
fn blit_centred(canvas: &mut Canvas, overlay: &Canvas) {
    if overlay.width() > canvas.width() || overlay.height() > canvas.height() {
        let mut widened = Canvas::new(canvas.width().max(overlay.width()), canvas.height().max(overlay.height()));
        widened.blit(canvas, 0, 0);
        *canvas = widened;
    }
    let row = canvas.height().saturating_sub(overlay.height()) / 2;
    let col = canvas.width().saturating_sub(overlay.width()) / 2;
    canvas.blit(overlay, row, col);
}

/// Draws the status bar, the current screen (with a zoomed card's detail,
/// and/or the roll-result modal, blitted over it), the message row, and —
/// pinned to the bottom, always [`HAND_ROWS`] tall — the active side's
/// hand strip. Everything but the screen's own view is kept in full; the
/// view is what gets clipped from the bottom when the terminal is too
/// short for all of it, the same rule the status bar already followed
/// before the hand strip existed.
#[allow(clippy::too_many_arguments)]
fn draw(
    screen: &Screen,
    map: &WorldMap,
    layout: &MapLayout,
    cards: &CardCatalog,
    game: &Game,
    message: Option<&str>,
    hand_selected: &HandUi,
    zoomed: bool,
    modal: &VecDeque<Modal>,
    color: ColorMode,
) -> io::Result<()> {
    // A choice waiting only for its confirmation stays on the message row
    // (unless something more pressing replaced it) until confirmed or undone.
    let forced = game.forced_by().zip(game.card_in_play()).zip(game.forced_how()).map(|((host, card), how)| match how {
        PlayAs::Event if host == card => format!("{} was spent on operations — now its event (the opponent's) has to be played: press e", cards.card(card).name),
        _ if host == card => format!("{} has to be used for operations this action round — i, a or o", cards.card(card).name),
        PlayAs::Event => format!("{} puts {} in play — press e to play its event (it can't be skipped or taken back)", cards.card(host).name, cards.card(card).name),
        PlayAs::Either => format!("{} puts {} in play — play it now: e for its event, or i/a/o for its operations", cards.card(host).name, cards.card(card).name),
        PlayAs::Ops => format!("{} puts {} in play — an opponent's event, so use its operations (i/a/o)", cards.card(host).name, cards.card(card).name),
    });
    let held = game.awaiting_discard().map(|side| format!("Eagle/Bear has Landed — {side} may discard one card: Space discards the selected card · p keeps them all"));
    let headline = game.picking_headline().then(|| match (game.headline_pick_seen(), game.headline_other_has_chosen()) {
        (Some((other, card)), _) => format!("Man in Earth Orbit — {other} headlined {}; choose yours (space)", cards.card(card).name),
        (None, true) => format!("{} has chosen a headline card (hidden) — now {}: [ ] select, space choose", game.active().opponent(), game.active()),
        (None, false) => format!("{}: choose your headline card — [ ] select, space choose", game.active()),
    });
    let reminder = pending_choice_reminder(game, cards).or(held).or(headline).or(forced).or_else(|| {
        (hand_selected.peek && peeking_allowed(game) && !discard_gate_open(game))
            .then(|| format!("showing the {} hand (revealed) — v to return to your own", game.active().opponent()))
    });
    let message = message.or(reminder.as_deref());
    let board = game.board();
    let op = game.operation();
    let mut canvas = match screen {
        Screen::World { selected } => render_world_map(map, layout, board, Some(*selected), op),
        Screen::Region { region, selected } => render_region(map, layout, board, *region, Some(*selected), op),
        Screen::Country { selected, .. } => render_country(map, layout, board, *selected, op, ViewMode::Interactive),
    };

    let side = hand_side(game, hand_selected.peek);
    let status = game.status();
    let china = (status.china_card == side).then_some(status.china_card_face_up);
    let hand = game.hand(side);
    let item_count = hand.len() + china.is_some() as usize;
    let selected_idx = (item_count > 0).then(|| hand_selected.selected[side_index(side)].min(item_count - 1));
    let hand_canvas = match (game.forced_by(), game.card_in_play(), game.forced_how()) {
        (Some(host), Some(card), Some(how)) if host != card => render_forced_card(cards, card, host, how, game.active()),
        _ => render_hand(cards, hand, china, side, selected_idx, game.card_in_play_slot().filter(|_| side == game.active())),
    };

    if zoomed && let Some(id) = zoom_card(game, hand_selected) {
        let china_face_up = (id == CHINA_CARD).then_some(status.china_card_face_up);
        let card_canvas = render_card(cards, id, china_face_up);
        blit_centred(&mut canvas, &card_canvas);
    }

    // The modal queue sits on top of everything, including a zoomed card
    // — the two can't actually be open together (see `run`'s own doc),
    // but drawing it last keeps that true even if that ever changes.
    if let Some(front) = modal.front() {
        let queue_pos = (modal.len() > 1).then_some((1, modal.len()));
        let modal_canvas = match front {
            Modal::Roll(report) => render_roll_result(map, report, queue_pos),
            Modal::Score(result, vp_after) => render_scoring_result(map, cards, result, *vp_after, queue_pos),
            Modal::Headline(ussr, us, first, cancelled) => render_headline_reveal(cards, *ussr, *us, *first, *cancelled, queue_pos),
            Modal::FinalScoring(results, china, vp_after, winner) => render_final_scoring(results, *china, *vp_after, *winner, queue_pos),
            Modal::Event(result, vp_after, winner) => render_event_result(map, cards, result, *vp_after, *winner, queue_pos),
            Modal::War(result, vp_after, winner) => render_war_result(map, cards, result, *vp_after, *winner, queue_pos),
            Modal::Space(result, vp_after, winner) => render_space_result(cards, result, *vp_after, *winner, queue_pos),
            Modal::SpaceTrack => render_space_track_with_hint(game.status(), "Enter/Esc/⌫/t close"),
            Modal::Piles { tab, cursor, zoom } => {
                let list = pile_cards(game.hands(), *tab);
                match list.get((*cursor).min(list.len().saturating_sub(1))) {
                    Some(&id) if *zoom => render_card(cards, id, None),
                    _ => render_piles(cards, game.hands(), *tab, *cursor),
                }
            }
            Modal::Session => match game.operation() {
                Some(Operation::Event(e)) => render_event_session(cards, e, game.status()),
                _ => Canvas::new(0, 0),
            },
            Modal::TrapConfirm(card) => match game.trap() {
                Some((effect, _)) => render_trap_confirm(effect, game.active(), cards.card(*card)),
                None => Canvas::new(0, 0),
            },
            Modal::Trap(result) => render_trap_result(cards, result, queue_pos),
            Modal::HeadlineConfirm(card) => render_headline_confirm(game.active(), cards.card(*card)),
            Modal::SpaceConfirm => match game.card_in_play() {
                Some(id) => render_space_confirm(game.status(), cards.card(id)),
                None => Canvas::new(0, 0),
            },
        };
        blit_centred(&mut canvas, &modal_canvas);
    }

    let card_in_play = game.card_in_play().map(|id| cards.card(id));
    let bar = render_status_bar_with(layout, board, game.status(), card_in_play, op, game.winner(), game.ops_after_event(), game.forced_by().zip(game.forced_how()).map(|(c, how)| (cards.card(c), how)), Some((game.hands().deck().len(), game.discards().len())), game.phase() == Phase::Setup, canvas.width());

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
