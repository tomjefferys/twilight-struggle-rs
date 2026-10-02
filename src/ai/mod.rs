//! The AI framework: the [`Ai`] trait an opponent implements, and
//! [`play_turn`], which drives one all the way through a single turn via
//! [`Game::legal_actions`]/[`Game::apply`] — the same forward-moves-only
//! surface `action.rs` defines, so an `Ai` never needs (and can't reach)
//! `Game`'s take-back methods.
//!
//! [`random::RandomAi`] is the first, simplest implementation: pick
//! uniformly among whatever's legal. A future, stronger opponent is just
//! another [`Ai`] impl — nothing here or in `action.rs` is specific to
//! randomness.

mod random;

pub use random::RandomAi;

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
/// [`Game::apply`]s it, stopping the moment [`Game::active`] changes —
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
pub fn play_turn(ai: &mut dyn Ai, game: &mut Game, map: &WorldMap, cards: &CardCatalog, dice: &mut Dice) -> Result<(), GameError> {
    let side = game.active();
    for _ in 0..MAX_ACTIONS_PER_TURN {
        if game.active() != side || game.winner().is_some() {
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
