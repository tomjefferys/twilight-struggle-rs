//! [`Action`]: one forward move in a turn, and [`Game::legal_actions`]/
//! [`Game::apply`], the enumeration/execution pair an AI opponent drives
//! instead of calling `Game`'s own methods directly. This is the "whole
//! surface a future AI opponent needs" promised by `game.rs`'s own doc,
//! made concrete.
//!
//! Deliberately **forward moves only** — [`Game::play_card`],
//! [`Game::begin`], [`Game::play_event`], [`Game::place`], [`Game::roll`],
//! [`Game::confirm`], and [`Game::pass`] — never the take-backs
//! ([`Game::undo`], [`Game::abandon`], [`Game::return_card`]) or
//! [`Game::cancel`]. Those exist so a human doesn't have to live with a
//! misclick, but they add nothing an AI needs: a take-back only ever
//! undoes an action that's still in this same list to begin with (so an
//! AI can simply not have chosen it), and `cancel`'s two board outcomes
//! are already reachable through `confirm` alone — an uncommitted
//! placement cancelled is the same board as one confirmed with nothing
//! placed, and a realignment/coup has no pending board state for
//! `cancel` to discard in the first place. Leaving them out keeps the list
//! non-redundant and guarantees something stronger: every action in it
//! either spends ops or closes/opens a step, so a random walk through
//! repeated `legal_actions`/`apply` calls can never stall (while the game
//! goes on — see [`Game::legal_actions`]'s own doc for the one exception)
//! — it always reaches a `Confirm` or `Pass` that hands the turn over, in
//! a bounded number of steps.

use crate::cards::{CardCatalog, CardId, CHINA_CARD};
use crate::country::CountryId;
use crate::dice::Dice;
use crate::events;
use crate::events::choice::Sign;
use crate::game::{Game, GameError, OperationKind, Trap};
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
    /// Resolve the card in play's own event — [`Game::play_event`]. Only
    /// offered when [`crate::events::is_implemented`] recognises it.
    Event,
    /// Spend the card in play on a space race attempt — [`Game::space`].
    /// Offered only when [`crate::space::check`] allows it.
    Space,
    /// Place one point of influence here — [`Game::place`]; in an open
    /// event, the `+` step (add influence there).
    Place(CountryId),
    /// The `-` step of an open event: remove influence here —
    /// [`Game::unplace`]. Offered only where it's a *forward* step (never
    /// as a mere take-back, for the same reason `undo` isn't an action).
    Unplace(CountryId),
    /// Choose which way to play an open multi-mode event (0-based) —
    /// [`Game::choose_mode`].
    ChooseMode(u8),
    /// Throw the open event's roll-off (Summit) — [`Game::roll_contest`].
    RollContest,
    /// Resolve one realignment roll, or a coup's one attempt, against this
    /// country — [`Game::roll`].
    Roll(CountryId),
    /// Close the open operation, committing whatever it did, and hand the
    /// turn to the other side — [`Game::confirm`].
    Confirm,
    /// A trapped side's action round: discard this card and roll to escape — [`Game::escape_trap`].
    Escape(CardId),
    /// Pick this card from the hand as the headline of the turn — [`Game::headline`].
    Headline(CardId),
    /// Settle what the last action round set off (NORAD) or the end of the turn — [`Game::settle`].
    Settle,
    /// The Eagle/Bear has Landed perk at the end of the turn: discard this held card, or `None`
    /// to keep them all — [`Game::discard_held`].
    DiscardHeld(Option<CardId>),
    /// Forfeit the turn with no card played — [`Game::pass`] — or, after a
    /// card's event, skip the operation it allowed.
    Pass,
}

impl Game {
    /// Every legal action for whoever's active right now, in a fixed,
    /// deterministic order (hand order, then [`WorldMap::iter`]'s own
    /// order) so the same game state always lists the same actions in the
    /// same order — a seeded AI's choices stay reproducible.
    ///
    /// Empty once [`Game::winner`] is set — there's nothing left to do —
    /// and otherwise never empty: with no card in play there's always at
    /// least `Pass` (and a card to play, unless the hand is somehow empty
    /// and the China Card isn't available — still safe, `Pass` alone
    /// covers it); with a card in play but no operation, a scoring card
    /// offers only `Event` (it has no ops `Begin` could spend) and every
    /// other card offers all three `Begin` kinds, plus `Event` too when
    /// [`events::is_implemented`] recognises it (whichever side's card it is);
    /// with an operation open, `Confirm` is always
    /// legal even if nothing on the board can be touched yet — except an
    /// open *event*, whose `Confirm` waits until it has been carried out as
    /// fully as it can be (see [`crate::events::choice`]), and which is
    /// offered to [`Game::decider`] (the event's chooser), not necessarily
    /// the active side.
    pub fn legal_actions(&self, map: &WorldMap, cards: &CardCatalog) -> Vec<Action> {
        if self.winner().is_some() {
            return Vec::new();
        }

        let mut actions = Vec::new();
        if self.settlement_due() {
            return vec![Action::Settle];
        }
        if self.picking_headline() {
            return self.headline_candidates(self.status().active).into_iter().map(Action::Headline).collect();
        }
        if let Some(side) = self.awaiting_discard() {
            let mut actions = vec![Action::DiscardHeld(None)];
            actions.extend(self.hand(side).iter().map(|&c| Action::DiscardHeld(Some(c))));
            return actions;
        }

        match self.operation() {
            Some(Operation::Influence(p)) => {
                for (id, _) in map.iter() {
                    if p.can_place(map, id) {
                        actions.push(Action::Place(id));
                    }
                }
                actions.push(Action::Confirm);
            }
            Some(Operation::Event(e)) if e.needs_roll() => actions.push(Action::RollContest),
            // Marking cards (Ask Not, Tehran): each unmarked card is a forward step; confirming is always allowed.
            Some(Operation::Event(e)) if e.is_multi() => {
                for i in 0..e.pile().len() {
                    if !e.is_marked(i) {
                        actions.push(Action::ChooseMode(i as u8));
                    }
                }
                actions.push(Action::Confirm);
            }
            Some(Operation::Event(e)) => {
                // Whoever `Game::decider` names — the event's chooser — is
                // the one these are offered to. Every forward step spends a
                // finite budget, so a random walk still reaches `Confirm`.
                if e.mode().is_none() {
                    for i in 0..e.modes().len() {
                        actions.push(Action::ChooseMode(i as u8));
                    }
                } else {
                    for (id, sign) in e.forward_steps(map) {
                        actions.push(match sign {
                            Sign::Plus => Action::Place(id),
                            Sign::Minus => Action::Unplace(id),
                        });
                    }
                    if e.is_complete() {
                        actions.push(Action::Confirm);
                    }
                }
            }
            Some(Operation::Realign(r)) => {
                for (id, _) in map.iter() {
                    if r.can_afford(map, id) && r.is_legal_target(map, self.board(), id) {
                        actions.push(Action::Roll(id));
                    }
                }
                actions.push(Action::Confirm);
            }
            Some(Operation::War(w)) => {
                // Rolling on a target is the whole event — no `Confirm`.
                for id in events::war::eligible_targets(map, w.card()) {
                    // (not one a lasting event shields, like Brush War's NATO-protected Europe)
                    if w.is_legal_target(map, id) {
                        actions.push(Action::Roll(id));
                    }
                }
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
                if let Some(grant) = self.ops_after_event() {
                    // The event is done; only the operation it allowed (or skipping it) is left.
                    for kind in [OperationKind::Influence, OperationKind::Realign, OperationKind::Coup] {
                        if grant.allows(kind) {
                            actions.push(Action::Begin(kind));
                        }
                    }
                    actions.push(Action::Pass);
                } else if self.forced_how() == Some(crate::events::PlayAs::Event) {
                    // A card another event put into play: its event is all that can happen.
                    actions.push(Action::Event);
                } else if let Some(id) = self.card_in_play() {
                    if self.event_playable(cards, id) && self.forced_how() != Some(crate::events::PlayAs::Ops) {
                        actions.push(Action::Event);
                    }
                    // A scoring card has no ops for `Begin` to spend —
                    // `Event` (pushed above, since every scoring card is
                    // implemented) is the only way to play one. Any other
                    // card can offer both.
                    if self.can_space() {
                        actions.push(Action::Space);
                    }
                    if !cards.card(id).scoring {
                        actions.push(Action::Begin(OperationKind::Influence));
                        actions.push(Action::Begin(OperationKind::Realign));
                        // Cuban Missile Crisis would lose the game for a coup.
                        if !self.status().effects.coup_forbidden(self.active()) {
                            actions.push(Action::Begin(OperationKind::Coup));
                        }
                    }
                } else if let Some((_, trap)) = self.trap() {
                    // A trapped round is spent escaping (or playing scoring cards, or skipped) — even
                    // when Missile Envy is owed, which is simply still owed afterwards.
                    match trap {
                        Trap::Escape(candidates) => actions.extend(candidates.into_iter().map(Action::Escape)),
                        Trap::PlayScoring => actions.extend(self.hand(self.active()).iter().filter(|&&c| events::scoring::is_scoring_card(c)).map(|&c| Action::PlayCard(c))),
                        Trap::Skip => actions.push(Action::Pass),
                    }
                } else if let Some((_, forced)) = self.status().forced_play.filter(|&(s, _)| s == self.active()) {
                    // Missile Envy's new holder has to spend it on operations.
                    match self.hand(self.active()).iter().copied().find(|c| c.number() == forced) {
                        Some(card) => actions.push(Action::PlayCard(card)),
                        None => actions.push(Action::Pass),
                    }
                } else {
                    let side = self.active();
                    for &id in self.hand(side) {
                        actions.push(Action::PlayCard(id));
                    }
                    if self.status().china_card == side && self.status().china_card_face_up {
                        actions.push(Action::PlayCard(CHINA_CARD));
                    }
                    if actions.is_empty() {
                        actions.push(Action::Pass);
                    }
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
            Action::Event => self.play_event_with(map, cards, dice).map(|_| ()),
            Action::Place(id) => self.place(map, id).map(|_| ()),
            Action::Unplace(id) => self.unplace(map, id),
            Action::ChooseMode(i) => self.choose_mode(map, i as usize),
            Action::Roll(id) => self.roll(map, id, dice).map(|_| ()),
            Action::RollContest => self.roll_contest(map, dice).map(|_| ()),
            Action::Confirm => self.confirm().map(|_| ()),
            Action::Space => self.space(dice).map(|_| ()),
            Action::Escape(card) => self.escape_trap(dice, card).map(|_| ()),
            Action::Settle => {
                self.settle(map, cards, dice);
                Ok(())
            }
            Action::Headline(card) => self.headline(cards, card),
            Action::DiscardHeld(card) => self.discard_held(card),
            Action::Pass => self.pass(),
        }
    }
}
