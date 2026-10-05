//! [`HeuristicAi`]: a cheap rule-of-thumb opponent. It scores positions with
//! [`evaluate`](super::eval::evaluate) and picks moves by looking at where they lead on a copy
//! of the game ([`Game::lookahead`]):
//!
//! - *Small* decisions (place a point, roll at a country, confirm) are **greedy**: try each on a
//!   copy, keep the best-scoring. A die is sampled with a few fixed seeds, never the real dice.
//! - *Big* decisions (which card, as event or operation, which mode, escape) are settled by
//!   **rollout**: play the option out greedily to the end of the card and score that.
//! - The headline is chosen from a static card score, since the opponent's pick is hidden.
//!
//! It sees the deck and hands as they are (a lookahead copy has no hidden information), which a
//! "vaguely sensible" bot can live with.

use crate::action::Action;
use crate::cards::{side_of, CardCatalog, CardSide};
use crate::country::Superpower;
use crate::dice::Dice;
use crate::events::scoring;
use crate::game::{Game, Phase};
use crate::map::WorldMap;
use crate::ops::{self, Operation};

use super::eval::evaluate;
use super::random::sensible;
use super::Ai;

/// How many sampled dice a roll candidate is averaged over.
const ROLL_SAMPLES: u64 = 4;
/// A defensive cap on steps in one rollout.
const MAX_ROLLOUT_STEPS: usize = 150;
/// How deep decisions nest inside a rollout: card → how to play it → which mode.
const DECISION_DEPTH: u8 = 2;
/// Headline: a card's own scoring swing counts this many times over.
const HEADLINE_SCORING: f32 = 2.0;

pub struct HeuristicAi {
    rng: Dice,
}

impl HeuristicAi {
    /// An AI whose tie-breaks are fully determined by `seed`.
    pub fn from_seed(seed: u64) -> Self {
        HeuristicAi { rng: Dice::from_seed(seed) }
    }

    pub fn from_entropy() -> Self {
        HeuristicAi { rng: Dice::from_entropy() }
    }
}

/// A choice between whole ways of playing, settled by rollout rather than one step.
fn is_decision(action: &Action) -> bool {
    matches!(
        action,
        Action::PlayCard(_) | Action::Begin(_) | Action::Event | Action::Space | Action::Pass | Action::ChooseMode(_) | Action::Escape(_) | Action::DiscardHeld(_)
    )
}

/// Applies `action` on a fresh copy of `game`, if the copy accepts it.
fn after(game: &Game, action: Action, map: &WorldMap, cards: &CardCatalog, seed: u64) -> Option<Game> {
    let mut copy = game.lookahead();
    copy.apply(action, map, cards, &mut Dice::from_seed(seed)).ok()?;
    Some(copy)
}

/// Where a rollout stops: the card is spent, the turn has moved on, or the game is over.
struct Root {
    me: Superpower,
    active: Superpower,
    phase: Phase,
}

impl Root {
    fn over(&self, game: &Game) -> bool {
        game.winner().is_some()
            || game.active() != self.active
            || game.phase() != self.phase
            || (game.card_in_play().is_none() && game.operation().is_none())
    }
}

/// What a candidate step is worth to whoever is choosing it, and what the resulting position is
/// worth to `me`.
struct Scored {
    action: Action,
    for_me: f32,
}

/// Whether to pick the best candidate for `me` or the worst (an opponent choosing mid-event).
fn best(candidates: Vec<Scored>, me_is_choosing: bool) -> Option<Scored> {
    let key = |s: &Scored| if me_is_choosing { s.for_me } else { -s.for_me };
    candidates.into_iter().max_by(|a, b| key(a).total_cmp(&key(b)))
}

/// The micro candidates worth rolling at: only a country where there is something to win.
fn worth_rolling(game: &Game, map: &WorldMap, id: crate::country::CountryId) -> bool {
    let side = game.ops_side();
    match game.operation() {
        Some(Operation::Realign(_)) => {
            let odds = ops::odds(map, game.board(), id, side);
            game.board().influence(id, side.opponent()) > 0 && odds.removed_36ths > odds.lost_36ths
        }
        Some(Operation::Coup(_)) => game.board().influence(id, side.opponent()) > 0,
        _ => true,
    }
}

/// Plays `game` (already holding the decision's first move) out to `root`'s stopping point and
/// scores it for `root.me`. `depth` is how many nested decisions may still be compared by their
/// own rollouts; beyond it a decision is taken by one-step lookahead.
fn rollout(mut game: Game, root: &Root, map: &WorldMap, cards: &CardCatalog, depth: u8) -> f32 {
    for _ in 0..MAX_ROLLOUT_STEPS {
        if root.over(&game) {
            break;
        }
        let legal = game.legal_actions(map, cards);
        if legal.is_empty() {
            break;
        }
        let chooser = game.decider();
        let me_choosing = chooser == root.me;
        if legal.len() == 1 {
            if game.apply(legal[0], map, cards, &mut Dice::from_seed(1)).is_err() {
                break;
            }
            continue;
        }
        if depth > 0 && legal.iter().any(is_decision) {
            let candidates: Vec<Scored> = legal
                .iter()
                .filter_map(|&a| {
                    let next = after(&game, a, map, cards, 1)?;
                    Some(Scored { action: a, for_me: rollout(next, root, map, cards, depth - 1) })
                })
                .collect();
            return best(candidates, me_choosing).map_or_else(|| evaluate(&game, map, cards, root.me), |s| s.for_me);
        }
        match greedy_step(&game, map, cards, chooser, &legal).and_then(|a| after_into(&mut game, a, map, cards)) {
            Some(()) => {}
            None => break,
        }
    }
    evaluate(&game, map, cards, root.me)
}

/// Applies `action` to `game` itself (the chosen step of a rollout).
fn after_into(game: &mut Game, action: Action, map: &WorldMap, cards: &CardCatalog) -> Option<()> {
    game.apply(action, map, cards, &mut Dice::from_seed(1)).ok()
}

/// The one-step-lookahead pick among `legal`, scoring each resulting position for `chooser`.
fn greedy_step(game: &Game, map: &WorldMap, cards: &CardCatalog, chooser: Superpower, legal: &[Action]) -> Option<Action> {
    let mut best: Option<(f32, Action)> = None;
    for &action in legal {
        let is_roll = matches!(action, Action::Roll(_) | Action::RollContest);
        if let Action::Roll(id) = action
            && !worth_rolling(game, map, id)
        {
            continue;
        }
        let seeds = if is_roll { ROLL_SAMPLES } else { 1 };
        let mut total = 0.0;
        let mut n = 0.0;
        for seed in 1..=seeds {
            if let Some(next) = after(game, action, map, cards, seed) {
                total += evaluate(&next, map, cards, chooser);
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        let score = total / n;
        // `>` keeps the earliest of equals, and `Confirm` is listed last: a step that gains
        // nothing still beats stopping, since ops left unspent are lost anyway.
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, action));
        }
    }
    // Every candidate refused or skipped: stopping is always an option in an operation.
    best.map(|(_, a)| a).or_else(|| legal.iter().copied().find(|a| matches!(a, Action::Confirm)))
}

/// A card in hand as a headline: its ops (plus a bonus for its own side's event), or, for a
/// scoring card, twice what it would pay us now.
fn headline_score(game: &Game, map: &WorldMap, cards: &CardCatalog, side: Superpower, card: crate::cards::CardId) -> f32 {
    let c = cards.card(card);
    if c.scoring {
        let swing = scoring::resolve(map, game.board(), &game.status().lasting, card).map_or(0.0, |r| r.vp_delta as f32);
        let mine = if side == Superpower::Us { swing } else { -swing };
        return HEADLINE_SCORING * mine;
    }
    let ops = c.ops as f32;
    match c.side {
        CardSide::Neutral => ops,
        s if s == side_of(side) => ops + 2.0,
        _ => -ops,
    }
}

impl Ai for HeuristicAi {
    fn choose(&mut self, game: &Game, map: &WorldMap, cards: &CardCatalog, legal: &[Action]) -> Action {
        let options = sensible(game, map, cards, legal);
        if options.len() == 1 {
            return options[0];
        }
        let me = game.decider();

        // The headline: a static pick, the opponent's is hidden.
        if options.iter().all(|a| matches!(a, Action::Headline(_))) {
            let score = |a: &Action| match a {
                Action::Headline(c) => headline_score(game, map, cards, me, *c),
                _ => f32::MIN,
            };
            return *options.iter().max_by(|a, b| score(a).total_cmp(&score(b))).unwrap();
        }

        let root = Root { me, active: game.active(), phase: game.phase() };
        if options.iter().any(is_decision) {
            let candidates: Vec<Scored> = options
                .iter()
                .filter_map(|&a| {
                    let next = after(game, a, map, cards, 1)?;
                    Some(Scored { action: a, for_me: rollout(next, &root, map, cards, DECISION_DEPTH - 1) })
                })
                .collect();
            // Ties (and an all-refused list) fall back to a random pick among the options.
            if let Some(top) = best(candidates, true) {
                return top.action;
            }
            return options[self.rng.index(options.len())];
        }

        greedy_step(game, map, cards, me, &options).unwrap_or(options[self.rng.index(options.len())])
    }
}
