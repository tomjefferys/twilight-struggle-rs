//! [`RandomAi`]: picks uniformly among whatever [`Game::legal_actions`]
//! offers. The simplest possible [`Ai`], and the first one — a baseline
//! opponent to play against (and, later, to measure a smarter one
//! against) rather than a serious one of its own.
//!
//! [`RandomAi::careful`] is the same random walk with two pieces of rules-awareness, enough
//! for a game to last past its first few turns instead of ending on a scoring card held at
//! the turn's end or a DEFCON suicide: it plays a scoring card when the rounds left only just
//! cover the ones it holds, and never takes a move that (as far as a look ahead on a copy of
//! the game can tell) hands the opponent the game through DEFCON.

use crate::action::Action;
use crate::cards::CardCatalog;
use crate::dice::Dice;
use crate::game::{Game, VictoryReason};
use crate::map::WorldMap;
use crate::ops::Operation;

use super::Ai;

/// Chooses a uniformly random legal action every time (unless made [`careful`](RandomAi::careful)).
/// Picking "confirm an operation" and "place one more point" with equal weight means it often
/// ends an operation almost as soon as it opens one — a known property of
/// uniform-random play, not a bug, and good enough for a first opponent to
/// test the framework against.
pub struct RandomAi {
    rng: Dice,
    careful: bool,
}

impl RandomAi {
    /// A `RandomAi` whose choices are fully determined by `seed` — the
    /// same seed always makes the same choices given the same sequence of
    /// legal-action lists.
    pub fn from_seed(seed: u64) -> Self {
        RandomAi { rng: Dice::from_seed(seed), careful: false }
    }

    /// A `RandomAi` seeded from the system clock, for real play.
    pub fn from_entropy() -> Self {
        RandomAi { rng: Dice::from_entropy(), careful: false }
    }

    /// The same AI, but one that avoids the two ways a random player loses at once (see the
    /// module doc). Still random among whatever is left.
    pub fn careful(mut self) -> Self {
        self.careful = true;
        self
    }
}

/// How many of `game`'s action rounds the side to act still has, this one included.
fn rounds_left(game: &Game) -> u8 {
    let status = game.status();
    (status.rounds_for(game.active()) + 1).saturating_sub(status.action_round)
}

/// The actions worth considering: with a scoring card that must be played now (as many held as
/// rounds left), only those; and without any that would lose to DEFCON at once. Falls back to
/// everything when a filter would leave nothing.
fn sensible(game: &Game, map: &WorldMap, cards: &CardCatalog, legal: &[Action]) -> Vec<Action> {
    let side = game.active();
    let scoring = |c| cards.card(c).scoring;
    let held = game.hand(side).iter().filter(|&&c| scoring(c)).count() as u8;
    if held > 0 && held >= rounds_left(game) {
        let must: Vec<Action> = legal.iter().copied().filter(|a| matches!(a, Action::PlayCard(c) if scoring(*c))).collect();
        if !must.is_empty() {
            return must;
        }
    }
    // A DEFCON loss is deterministic given the move, so look one move ahead on a copy — only
    // worth the cost when DEFCON is low enough to matter.
    if game.status().defcon > 3 {
        return legal.to_vec();
    }
    let risky = |a: &Action| {
        let tries = match a {
            Action::Event => true,
            Action::Roll(_) => matches!(game.operation(), Some(Operation::Coup(_))),
            _ => false,
        };
        if !tries {
            return false;
        }
        let mut ahead = game.lookahead();
        ahead.apply(*a, map, cards, &mut Dice::from_seed(0)).is_ok()
            && ahead.winner().is_some_and(|v| matches!(v.reason, VictoryReason::Defcon | VictoryReason::CubanMissileCrisis) && v.side == Some(game.active().opponent()))
    };
    let safe: Vec<Action> = legal.iter().copied().filter(|a| !risky(a)).collect();
    if safe.is_empty() { legal.to_vec() } else { safe }
}

impl Ai for RandomAi {
    fn choose(&mut self, game: &Game, map: &WorldMap, cards: &CardCatalog, legal: &[Action]) -> Action {
        if !self.careful {
            return legal[self.rng.index(legal.len())];
        }
        let options = sensible(game, map, cards, legal);
        options[self.rng.index(options.len())]
    }
}
