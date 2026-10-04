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
pub use effects::{ChinaTransfer, Contest, ContestRoll, EffectResult, Reveal};
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

/// The operations a card's event also lets its player conduct with the
/// card's own ops value, once the event has resolved (ABM Treaty, KAL-007,
/// Glasnost, CIA Created, "Lone Gunman", Junta, Che, ...). The card stays
/// in play until they're done — or skipped with `Game::pass`. A grant may
/// confine coups and realignments to a set of countries (`scope`), and
/// Che's carries a follow-up: a second coup, in a different country, if
/// the first removed any US influence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpsGrant {
    pub influence: bool,
    pub realign: bool,
    pub coup: bool,
    /// Where a coup or realignment may land.
    pub scope: choice::Where,
    /// Completes "isn't …" when a target is refused, and the description.
    pub scope_label: &'static str,
    /// A country already used, which can't be the target again.
    pub exclude: Option<crate::country::CountryId>,
    /// A second coup is allowed if this one removes US influence (Che).
    pub follow_up: bool,
    /// The operation is worth this many ops instead of the card's own (Olympic Games' boycott: 4).
    pub ops: Option<u8>,
}

impl OpsGrant {
    const fn kinds(influence: bool, realign: bool, coup: bool) -> OpsGrant {
        OpsGrant { influence, realign, coup, scope: choice::Where::Everywhere, scope_label: "", exclude: None, follow_up: false, ops: None }
    }

    /// Any operation.
    pub const ANY: OpsGrant = OpsGrant::kinds(true, true, true);
    /// Influence or realignment only (KAL-007, Glasnost).
    pub const NO_COUP: OpsGrant = OpsGrant::kinds(true, true, false);

    /// Coups and realignments (not placement) within `scope`.
    const fn coup_or_realign_in(scope: choice::Where, label: &'static str) -> OpsGrant {
        OpsGrant { scope, scope_label: label, ..OpsGrant::kinds(false, true, true) }
    }

    /// The same grant, worth `ops` operation points rather than the card's own.
    pub const fn with_ops(self, ops: u8) -> OpsGrant {
        OpsGrant { ops: Some(ops), ..self }
    }

    pub fn allows(self, kind: crate::game::OperationKind) -> bool {
        match kind {
            crate::game::OperationKind::Influence => self.influence,
            crate::game::OperationKind::Realign => self.realign,
            crate::game::OperationKind::Coup => self.coup,
        }
    }

    /// The target restriction a coup or realignment opened under this
    /// grant carries, if it has one.
    pub fn target_scope(self) -> Option<crate::ops::TargetScope> {
        (self.scope != choice::Where::Everywhere || self.exclude.is_some())
            .then_some(crate::ops::TargetScope { place: self.scope, exclude: self.exclude, label: self.scope_label })
    }

    /// What may be done, for a message.
    pub fn describe(self) -> String {
        let kinds: Vec<&str> = [(self.influence, "placing influence"), (self.realign, "realigning"), (self.coup, "a coup")]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect();
        let mut text = if kinds.len() == 3 { "any operation".to_string() } else { kinds.join(" or ") };
        if !self.scope_label.is_empty() {
            text = format!("{text} {}", self.scope_label);
        }
        if let Some(ops) = self.ops {
            text.push_str(&format!(" as if the card were worth {ops} ops"));
        }
        if self.follow_up {
            text.push_str(" (and a second coup in a different country if it removes US influence)");
        }
        text
    }
}

const AMERICAS: choice::Where = choice::Where::Any(&[choice::Where::Region(crate::country::Region::CentralAmerica), choice::Where::Region(crate::country::Region::SouthAmerica)]);
const CHE_REGIONS: choice::Where = choice::Where::Any(&[
    choice::Where::Region(crate::country::Region::CentralAmerica),
    choice::Where::Region(crate::country::Region::SouthAmerica),
    choice::Where::Region(crate::country::Region::Africa),
]);

/// The operations `card`'s event grants `player` after it resolves, if any.
/// Judged against the board as it stood when the event was played; only the
/// card's own side gets them (the VP/DEFCON text applies whoever plays it,
/// but "the US may place influence…" does not).
pub fn ops_grant(map: &WorldMap, board: &Board, removed: &[CardId], card: CardId, player: crate::country::Superpower) -> Option<OpsGrant> {
    use crate::country::Region;
    use crate::country::Superpower::{Us, Ussr};
    match card.0 {
        26 if player == Us => Some(OpsGrant::ANY),
        47 => Some(OpsGrant::coup_or_realign_in(AMERICAS, "in Central or South America")),
        57 => Some(OpsGrant::ANY),
        62 if player == Ussr => Some(OpsGrant::ANY),
        89 if player == Us && map.id_by_name("South Korea").is_some_and(|id| board.is_controlled_by(map, id, Us)) => Some(OpsGrant::NO_COUP),
        90 if player == Ussr && removed.contains(&CardId(87)) => Some(OpsGrant::NO_COUP),
        91 if player == Ussr => Some(OpsGrant { realign: false, scope: choice::Where::AdjacentTo("Nicaragua"), scope_label: "adjacent to Nicaragua", ..OpsGrant::kinds(false, false, true) }),
        96 if player == Us => Some(OpsGrant::coup_or_realign_in(choice::Where::Region(Region::Europe), "in Europe")),
        107 if player == Ussr => Some(OpsGrant {
            realign: false,
            scope: choice::Where::NonBattleground(&CHE_REGIONS),
            scope_label: "in a non-battleground country in Central America, South America or Africa",
            follow_up: true,
            ..OpsGrant::kinds(false, false, true)
        }),
        _ => None,
    }
}

/// Why a card's event can't be played right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    /// The event of card `by` has already happened and bars it (Camp David
    /// vs Arab-Israeli War, The Iron Lady vs Socialist Governments, ...).
    Prevented { by: CardId },
    /// It needs one of these cards' events to have happened first.
    Requires { any_of: &'static [CardId] },
    /// The card's event can't be played in the Late War (turn 8 on).
    LateWar,
}

/// The first turn of the Late War.
pub const LATE_WAR_TURN: u8 = 8;

/// Cards whose event can't be played in the Late War (The Cambridge Five).
const NOT_IN_LATE_WAR: &[u8] = &[104];

/// [`blocked`], plus the clauses that depend on the turn.
pub fn blocked_at(card: CardId, removed: &[CardId], turn: u8) -> Option<Blocked> {
    if NOT_IN_LATE_WAR.contains(&card.0) && turn >= LATE_WAR_TURN {
        return Some(Blocked::LateWar);
    }
    blocked(card, removed)
}

/// Whether resolving `card`'s event draws on chance (Terrorism's random discard, , and so
/// needs `Game::play_event_with`'s dice.
pub fn needs_dice(card: CardId) -> bool {
    matches!(card.0, 5 | 92)
}

/// Cards whose event is barred once another card's event has happened.
const PREVENTED_BY: &[(u8, u8)] = &[(7, 83), (13, 65), (55, 96), (56, 110), (59, 97), (61, 86)];

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
