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
//! `events::choice` is the third: cards that make a player pick countries.
//! They don't resolve in one call; they open an [`EventChoice`] that
//! `Game::play_event` carries as `Operation::Event` until confirmed.
//!
//! `events::effects` is the second stage: nineteen cards whose text only
//! moves influence, VP, or DEFCON by fixed amounts. `CARDS.md` (repo
//! root) tracks which cards are implemented; a test keeps it in step
//! with [`is_implemented`].

pub mod choice;
pub mod effects;
pub mod scoring;
pub mod war;

pub use choice::EventChoice;
pub use effects::EffectResult;
pub use scoring::ScoringResult;
pub use war::{War, WarResult};

use crate::board::Board;
use crate::cards::CardId;
use crate::map::WorldMap;
use crate::status::GameStatus;

/// What resolving a card's event actually produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventOutcome {
    Scoring(ScoringResult),
    Effect(EffectResult),
    /// A choice card's event has opened a session for `chooser` to work
    /// through — nothing has changed yet.
    Pending { card: CardId, chooser: crate::country::Superpower },
}

/// Whether `card`'s event is implemented yet — what
/// [`crate::game::Game::play_event`] checks before calling [`resolve`],
/// and what [`crate::game::Game::legal_actions`] checks before
/// offering `Action::Event` for a card in play.
pub fn is_implemented(card: CardId) -> bool {
    scoring::is_scoring_card(card) || effects::is_effect_card(card) || choice::is_choice_card(card) || war::is_war_card(card)
}

/// The card whose event stops `card`'s from being played, if it has
/// already been played (i.e. is in `removed`) — the first modelled
/// "prevents" clause: #65 Camp David Accords bars #13 Arab-Israeli War.
pub fn is_prevented(card: CardId, removed: &[CardId]) -> Option<CardId> {
    match card.0 {
        13 => removed.iter().copied().find(|c| c.0 == 65),
        _ => None,
    }
}

/// Resolves `card`'s event against `(map, board)` — `None` for a card
/// [`is_implemented`] doesn't recognise. Read-only: applying the result
/// (VP, discard, victory) is `Game::play_event`'s job, not this
/// function's — the same split `ops::realign::resolve` keeps between
/// computing a roll's outcome and `Game::roll` writing it to the board.
///
/// Choice cards ([`choice::is_choice_card`]) are *not* resolved here —
/// they need a session, so `resolve` returns `None` for them too and
/// `Game::play_event` checks [`choice::is_choice_card`] first.
pub(crate) fn resolve(map: &WorldMap, board: &Board, status: &GameStatus, card: CardId) -> Option<EventOutcome> {
    scoring::resolve(map, board, card)
        .map(EventOutcome::Scoring)
        .or_else(|| effects::resolve(map, board, status, card).map(EventOutcome::Effect))
}
