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

/// Why a card's event can't be played right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    /// The event of card `by` has already happened and bars it (Camp David
    /// vs Arab-Israeli War, The Iron Lady vs Socialist Governments, ...).
    Prevented { by: CardId },
    /// It needs one of these cards' events to have happened first.
    Requires { any_of: &'static [CardId] },
}

/// Cards whose event is barred once another card's event has happened.
const PREVENTED_BY: &[(u8, u8)] = &[(7, 83), (13, 65), (56, 110), (59, 97)];

/// Cards whose event may only be played after one of the listed cards'.
const REQUIRES: &[(u8, &[CardId])] = &[(21, &[CardId(16), CardId(23)]), (101, &[CardId(68)])];

/// Whether `card`'s event is barred, given the cards already removed from
/// the game (`removed` — a played event's card lands there, which is all
/// these prevents/requires clauses need to look at).
pub fn blocked(card: CardId, removed: &[CardId]) -> Option<Blocked> {
    if let Some(&(_, by)) = PREVENTED_BY.iter().find(|&&(n, _)| n == card.0)
        && removed.contains(&CardId(by))
    {
        return Some(Blocked::Prevented { by: CardId(by) });
    }
    if let Some(&(_, any_of)) = REQUIRES.iter().find(|&&(n, _)| n == card.0)
        && !any_of.iter().any(|c| removed.contains(c))
    {
        return Some(Blocked::Requires { any_of });
    }
    None
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
    scoring::resolve(map, board, &status.lasting, card)
        .map(EventOutcome::Scoring)
        .or_else(|| effects::resolve(map, board, status, card).map(EventOutcome::Effect))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_played_prevents_card_bars_its_target_and_a_requirement_needs_one_of_its_cards() {
        assert_eq!(blocked(CardId(7), &[]), None);
        assert_eq!(blocked(CardId(7), &[CardId(83)]), Some(Blocked::Prevented { by: CardId(83) }));
        assert_eq!(blocked(CardId(13), &[CardId(65)]), Some(Blocked::Prevented { by: CardId(65) }));
        assert_eq!(blocked(CardId(56), &[CardId(110)]), Some(Blocked::Prevented { by: CardId(110) }));
        assert_eq!(blocked(CardId(59), &[CardId(97)]), Some(Blocked::Prevented { by: CardId(97) }));
        assert!(matches!(blocked(CardId(21), &[]), Some(Blocked::Requires { .. })));
        assert_eq!(blocked(CardId(21), &[CardId(16)]), None);
        assert_eq!(blocked(CardId(21), &[CardId(23)]), None);
        assert!(matches!(blocked(CardId(101), &[CardId(16)]), Some(Blocked::Requires { .. })));
        assert_eq!(blocked(CardId(101), &[CardId(68)]), None);
    }
}
