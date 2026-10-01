//! [`RandomAi`]: picks uniformly among whatever [`Game::legal_actions`]
//! offers. The simplest possible [`Ai`], and the first one — a baseline
//! opponent to play against (and, later, to measure a smarter one
//! against) rather than a serious one of its own.

use crate::action::Action;
use crate::cards::CardCatalog;
use crate::dice::Dice;
use crate::game::Game;
use crate::map::WorldMap;

use super::Ai;

/// Chooses a uniformly random legal action every time. Picking "confirm an
/// operation" and "place one more point" with equal weight means it often
/// ends an operation almost as soon as it opens one — a known property of
/// uniform-random play, not a bug, and good enough for a first opponent to
/// test the framework against.
pub struct RandomAi {
    rng: Dice,
}

impl RandomAi {
    /// A `RandomAi` whose choices are fully determined by `seed` — the
    /// same seed always makes the same choices given the same sequence of
    /// legal-action lists.
    pub fn from_seed(seed: u64) -> Self {
        RandomAi { rng: Dice::from_seed(seed) }
    }

    /// A `RandomAi` seeded from the system clock, for real play.
    pub fn from_entropy() -> Self {
        RandomAi { rng: Dice::from_entropy() }
    }
}

impl Ai for RandomAi {
    fn choose(&mut self, _game: &Game, _map: &WorldMap, _cards: &CardCatalog, legal: &[Action]) -> Action {
        legal[self.rng.index(legal.len())]
    }
}
