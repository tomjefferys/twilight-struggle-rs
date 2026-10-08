//! The AI framework: the [`Ai`] trait an opponent implements, and
//! [`play_turn`], which drives one all the way through a single turn via
//! [`Game::legal_actions`]/[`Game::apply`] — the same forward-moves-only
//! surface `action.rs` defines, so an `Ai` never needs (and can't reach)
//! `Game`'s take-back methods.
//!
//! [`random::RandomAi`] is the first, simplest implementation: pick
//! uniformly among whatever's legal. [`HeuristicAi`] is the stronger one:
//! it scores positions ([`eval`]) and picks the moves that lead to the best
//! ones. Either is just an [`Ai`] impl — nothing here or in `action.rs` is
//! specific to either — and [`AiKind`] picks which to build.

mod eval;
mod heuristic;
mod random;
mod search;

pub use eval::evaluate;
pub use heuristic::HeuristicAi;
pub use random::RandomAi;
pub use search::{Budget, SearchAi};

/// Which opponent to play against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AiKind {
    /// Scores positions and plays the best move it can find ([`HeuristicAi`]).
    Heuristic,
    /// Determinized Monte Carlo search over whole action rounds ([`SearchAi`]): the strongest,
    /// and the one that takes a moment to think.
    #[default]
    Search,
    /// Random legal moves, with just enough care to finish a game ([`RandomAi::careful`]).
    Random,
}

impl AiKind {
    pub fn parse(word: &str) -> Option<AiKind> {
        match word.to_ascii_lowercase().as_str() {
            "heuristic" | "smart" => Some(AiKind::Heuristic),
            "search" | "strong" | "mcts" => Some(AiKind::Search),
            "random" => Some(AiKind::Random),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AiKind::Heuristic => "heuristic",
            AiKind::Search => "search",
            AiKind::Random => "random",
        }
    }

    /// Builds this kind of AI, seeded from `seed` (or the clock when `None`).
    pub fn build(self, seed: Option<u64>) -> Box<dyn Ai> {
        match (self, seed) {
            (AiKind::Search, Some(s)) => Box::new(SearchAi::from_seed(s)),
            (AiKind::Search, None) => Box::new(SearchAi::from_entropy()),
            (AiKind::Heuristic, Some(s)) => Box::new(HeuristicAi::from_seed(s)),
            (AiKind::Heuristic, None) => Box::new(HeuristicAi::from_entropy()),
            (AiKind::Random, Some(s)) => Box::new(RandomAi::from_seed(s).careful()),
            (AiKind::Random, None) => Box::new(RandomAi::from_entropy().careful()),
        }
    }
}

use crate::action::Action;
use crate::cards::CardCatalog;
use crate::dice::Dice;
use crate::game::{Game, GameError};
use crate::map::WorldMap;

/// Something that can play a superpower's turn by choosing among whatever
/// [`Game::legal_actions`] currently offers.
pub trait Ai {
    /// Picks one action from `legal` — always non-empty, always exactly
    /// the slice [`Game::legal_actions`] just returned for `game`'s
    /// current state. The implementation must return one of `legal`'s own
    /// elements; [`play_turn`] applies whatever comes back without
    /// re-checking it, so returning anything else would surface as an
    /// ordinary [`GameError`] from [`Game::apply`].
    fn choose(&mut self, game: &Game, map: &WorldMap, cards: &CardCatalog, legal: &[Action]) -> Action;

    /// Whether the next [`Ai::choose`] on this state is expected to take a noticeable moment
    /// (a search), so a front end can say the AI is thinking first.
    fn is_slow(&self, _game: &Game, _legal: &[Action]) -> bool {
        false
    }
}

/// A defensive cap on actions taken in one [`play_turn`] call. One turn's
/// own forward-moves-only structure (see `action.rs`'s module doc) already
/// guarantees this is never approached — a placement can touch at most
/// every country on the map before `Confirm` becomes the only option left,
/// and a realignment/coup/play-card/begin/pass step each end a phase
/// outright — so hitting this is a bug in an `Ai` impl (e.g. one that
/// doesn't always eventually choose `Confirm`/`Pass`), not a sign the game
/// itself can run long.
const MAX_ACTIONS_PER_TURN: usize = 1000;

/// Plays `ai`'s side's turn to completion: repeatedly lists
/// [`Game::legal_actions`], asks `ai` to [`Ai::choose`] one, and
/// [`Game::apply`]s it, stopping the moment [`Game::active`] or
/// [`Game::decider`] changes (the latter when an event hands its choices
/// to the other side) —
/// or, now that a scoring event can end the game mid-turn without ever
/// changing whose turn it technically is, the moment [`Game::winner`] is
/// set, since `legal_actions` would otherwise come back empty and
/// `choose` is documented to never see that. Works from any point
/// mid-turn (a card already played, an operation already open), not
/// just the very start of one.
///
/// Returns whatever [`Game::apply`] returned on its one failing call, if
/// any — `choose` returning something other than one of `legal`'s own
/// actions is the only way that can happen, since `legal_actions` only
/// ever lists moves `apply` accepts.
pub fn play_turn<A: Ai + ?Sized>(ai: &mut A, game: &mut Game, map: &WorldMap, cards: &CardCatalog, dice: &mut Dice) -> Result<(), GameError> {
    // `decider`, not `active`: an event's chooser is the card's own side,
    // so a turn can pass the move to the other side and back mid-turn.
    let side = game.decider();
    let phasing = game.active();
    for _ in 0..MAX_ACTIONS_PER_TURN {
        if game.active() != phasing || game.decider() != side || game.winner().is_some() {
            return Ok(());
        }
        let legal = game.legal_actions(map, cards);
        let action = ai.choose(game, map, cards, &legal);
        game.apply(action, map, cards, dice)?;
    }
    panic!(
        "play_turn exceeded {MAX_ACTIONS_PER_TURN} actions without ending {side}'s turn — \
         an Ai impl must always eventually choose Confirm or Pass"
    );
}
