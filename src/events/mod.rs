//! Card events: whatever playing a card for its *text* does, as opposed
//! to its ops value (which `Game::play_card`/`begin` already cover). This
//! is the first module to give any card's text real behaviour — nothing
//! above it (`cards.rs`, `game.rs`) reads a card's text at all.
//!
//! Scoring cards are the first stage: `events::scoring` resolves the
//! seven of them (rule 10.1) into a VP swing, purely as a function of
//! `(map, board, card)` — no session of its own, since a scoring card's
//! event has no choices to make and nothing to stage. [`resolve`] is the
//! single entry point [`crate::game::Game::play_event`] calls; later
//! stages (events with real choices — a country to target, an amount to
//! place) will add their own submodules and `EventOutcome` variants
//! alongside this one, but won't need to change this module's shape.
//!
//! `events::effects` is the second stage: nineteen cards whose text only
//! moves influence, VP, or DEFCON by fixed amounts. `CARDS.md` (repo
//! root) tracks which cards are implemented; a test keeps it in step
//! with [`is_implemented`].

pub mod effects;
pub mod scoring;

pub use effects::EffectResult;
pub use scoring::ScoringResult;

use crate::board::Board;
use crate::cards::CardId;
use crate::map::WorldMap;
use crate::status::GameStatus;

/// What resolving a card's event actually produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventOutcome {
    Scoring(ScoringResult),
    Effect(EffectResult),
}

/// Whether `card`'s event is implemented yet — what
/// [`crate::game::Game::play_event`] checks before calling [`resolve`],
/// and what [`crate::game::Game::legal_actions`] checks before
/// offering `Action::Event` for a card in play.
pub fn is_implemented(card: CardId) -> bool {
    scoring::is_scoring_card(card) || effects::is_effect_card(card)
}

/// Resolves `card`'s event against `(map, board)` — `None` for a card
/// [`is_implemented`] doesn't recognise. Read-only: applying the result
/// (VP, discard, victory) is `Game::play_event`'s job, not this
/// function's — the same split `ops::realign::resolve` keeps between
/// computing a roll's outcome and `Game::roll` writing it to the board.
pub(crate) fn resolve(map: &WorldMap, board: &Board, status: &GameStatus, card: CardId) -> Option<EventOutcome> {
    scoring::resolve(map, board, card)
        .map(EventOutcome::Scoring)
        .or_else(|| effects::resolve(map, board, status, card).map(EventOutcome::Effect))
}
