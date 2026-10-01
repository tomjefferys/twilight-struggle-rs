//! [`Action`]: one forward move in a turn, and [`Game::legal_actions`]/
//! [`Game::apply`], the enumeration/execution pair an AI opponent drives
//! instead of calling `Game`'s own methods directly. This is the "whole
//! surface a future AI opponent needs" promised by `game.rs`'s own doc,
//! made concrete.
//!
//! Deliberately **forward moves only** — [`Game::play_card`],
//! [`Game::begin`], [`Game::place`], [`Game::roll`], [`Game::confirm`], and
//! [`Game::pass`] — never the take-backs ([`Game::undo`], [`Game::abandon`],
//! [`Game::return_card`]) or [`Game::cancel`]. Those exist so a human doesn't
//! have to live with a misclick, but they add nothing an AI needs: a
//! take-back only ever undoes an action that's still in this same list to
//! begin with (so an AI can simply not have chosen it), and `cancel`'s two
//! board outcomes are already reachable through `confirm` alone — an
//! uncommitted placement cancelled is the same board as one confirmed with
//! nothing placed, and a realignment/coup has no pending board state for
//! `cancel` to discard in the first place. Leaving them out keeps the list
//! non-redundant and guarantees something stronger: every action in it
//! either spends ops or closes/opens a step, so a random walk through
//! repeated `legal_actions`/`apply` calls can never stall — it always
//! reaches a `Confirm` or `Pass` that hands the turn over, in a bounded
//! number of steps.

use crate::cards::{CardCatalog, CardId, CHINA_CARD};
use crate::country::CountryId;
use crate::dice::Dice;
use crate::game::{Game, GameError, OperationKind};
use crate::map::WorldMap;
use crate::ops::Operation;

/// One legal forward move — see the module doc for why this list is
/// deliberately not every method `Game` exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Play this card from the active side's hand (or the China Card, if
    /// it's theirs and face up) — [`Game::play_card`].
    PlayCard(CardId),
    /// Open an operation of this kind with the card already in play —
    /// [`Game::begin`].
    Begin(OperationKind),
    /// Place one point of influence here — [`Game::place`].
    Place(CountryId),
    /// Resolve one realignment roll, or a coup's one attempt, against this
    /// country — [`Game::roll`].
    Roll(CountryId),
    /// Close the open operation, committing whatever it did, and hand the
    /// turn to the other side — [`Game::confirm`].
    Confirm,
    /// Forfeit the turn with no card played — [`Game::pass`].
    Pass,
}

impl Game {
    /// Every legal action for whoever's active right now, in a fixed,
    /// deterministic order (hand order, then [`WorldMap::iter`]'s own
    /// order) so the same game state always lists the same actions in the
    /// same order — a seeded AI's choices stay reproducible.
    ///
    /// Never empty: with no card in play there's always at least `Pass`
    /// (and a card to play, unless the hand is somehow empty of non-scoring
    /// cards and the China Card isn't available — still safe, `Pass`
    /// alone covers it); with a card in play but no operation, all three
    /// `Begin` kinds are always legal; with an operation open, `Confirm` is
    /// always legal even if nothing on the board can be touched yet.
    pub fn legal_actions(&self, map: &WorldMap, cards: &CardCatalog) -> Vec<Action> {
        let mut actions = Vec::new();

        match self.operation() {
            Some(Operation::Influence(p)) => {
                for (id, _) in map.iter() {
                    if p.is_legal_target(map, id) && p.cost(map, id) <= p.remaining() {
                        actions.push(Action::Place(id));
                    }
                }
                actions.push(Action::Confirm);
            }
            Some(Operation::Realign(r)) => {
                if r.remaining() > 0 {
                    for (id, _) in map.iter() {
                        if r.is_legal_target(map, self.board(), id) {
                            actions.push(Action::Roll(id));
                        }
                    }
                }
                actions.push(Action::Confirm);
            }
            Some(Operation::Coup(c)) => {
                if c.result().is_none() {
                    for (id, _) in map.iter() {
                        if c.is_legal_target(map, self.board(), id) {
                            actions.push(Action::Roll(id));
                        }
                    }
                }
                actions.push(Action::Confirm);
            }
            None => {
                if self.card_in_play().is_some() {
                    actions.push(Action::Begin(OperationKind::Influence));
                    actions.push(Action::Begin(OperationKind::Realign));
                    actions.push(Action::Begin(OperationKind::Coup));
                } else {
                    let side = self.active();
                    for &id in self.hand(side) {
                        if !cards.card(id).scoring {
                            actions.push(Action::PlayCard(id));
                        }
                    }
                    if self.status().china_card == side && self.status().china_card_face_up {
                        actions.push(Action::PlayCard(CHINA_CARD));
                    }
                    actions.push(Action::Pass);
                }
            }
        }

        actions
    }

    /// Executes one action from [`Game::legal_actions`] — a thin dispatch
    /// onto the underlying `Game` method, discarding its own return value
    /// (an AI reads the result, if it cares, from [`Game::log`] afterwards,
    /// the same way a human reading the REPL's own prompt does). Logging
    /// happens exactly where it already does inside each of those methods;
    /// this adds none of its own.
    pub fn apply(&mut self, action: Action, map: &WorldMap, cards: &CardCatalog, dice: &mut Dice) -> Result<(), GameError> {
        match action {
            Action::PlayCard(id) => self.play_card(cards, id),
            Action::Begin(kind) => self.begin(kind),
            Action::Place(id) => self.place(map, id).map(|_| ()),
            Action::Roll(id) => self.roll(map, id, dice).map(|_| ()),
            Action::Confirm => self.confirm().map(|_| ()),
            Action::Pass => self.pass(),
        }
    }
}
