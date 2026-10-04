//! The Space Race (rule 6.4): eight boxes each side works along by
//! discarding an ops card and rolling at or under the box's number.
//!
//! Pure rules, like `ongoing.rs`: nothing here mutates a game.
//! [`crate::game::Game::space`] is what applies a [`SpaceResult`]. Box
//! perks are *derived* from the two markers ([`perk_holder`]) rather than
//! stored, so one is cancelled the moment the opponent reaches its box
//! with no extra state to keep in step.

use std::fmt;

use crate::cards::CardId;
use crate::country::Superpower;
use crate::status::GameStatus;

/// The last box on the track.
pub const MAX_BOX: u8 = 8;

/// A box's perk, held by whoever got there first until the opponent arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Perk {
    /// Box 2: two space attempts a turn instead of one.
    TwoAttempts,
    /// Box 4: the opponent must choose and show their headline card first.
    /// Tracked only — there is no headline phase yet.
    OpponentHeadlinesFirst,
    /// Box 6: may discard one held card at the end of the turn
    /// ([`crate::game::Game::discard_held`]).
    DiscardHeld,
    /// Box 8: may take eight action rounds in a turn.
    EightRounds,
}

impl Perk {
    pub const ALL: [Perk; 4] = [Perk::TwoAttempts, Perk::OpponentHeadlinesFirst, Perk::DiscardHeld, Perk::EightRounds];

    /// The box this perk belongs to.
    pub fn box_index(self) -> u8 {
        match self {
            Perk::TwoAttempts => 2,
            Perk::OpponentHeadlinesFirst => 4,
            Perk::DiscardHeld => 6,
            Perk::EightRounds => 8,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Perk::TwoAttempts => "2 space attempts a turn",
            Perk::OpponentHeadlinesFirst => "opponent headlines first",
            Perk::DiscardHeld => "may discard 1 held card",
            Perk::EightRounds => "8 action rounds",
        }
    }

    /// Whether the crate acts on this perk, or only displays it.
    pub fn is_enforced(self) -> bool {
        matches!(self, Perk::TwoAttempts | Perk::EightRounds | Perk::DiscardHeld)
    }
}

/// One box of the track (boxes 1-8; box 0 is the start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceBox {
    pub name: &'static str,
    /// Minimum ops on the card spent.
    pub ops: u8,
    /// The attempt succeeds on a die roll of this or less.
    pub max_roll: u8,
    pub vp_first: u8,
    pub vp_second: u8,
    pub perk: Option<Perk>,
}

/// Boxes 1-8, index 0 being box 1.
pub const TRACK: [SpaceBox; 8] = [
    SpaceBox { name: "Earth Satellite", ops: 2, max_roll: 3, vp_first: 2, vp_second: 1, perk: None },
    SpaceBox { name: "Animal in Space", ops: 2, max_roll: 4, vp_first: 0, vp_second: 0, perk: Some(Perk::TwoAttempts) },
    SpaceBox { name: "Man in Space", ops: 2, max_roll: 3, vp_first: 2, vp_second: 0, perk: None },
    SpaceBox { name: "Man in Earth Orbit", ops: 2, max_roll: 4, vp_first: 0, vp_second: 0, perk: Some(Perk::OpponentHeadlinesFirst) },
    SpaceBox { name: "Lunar Orbit", ops: 3, max_roll: 3, vp_first: 3, vp_second: 1, perk: None },
    SpaceBox { name: "Eagle/Bear has Landed", ops: 3, max_roll: 4, vp_first: 0, vp_second: 0, perk: Some(Perk::DiscardHeld) },
    SpaceBox { name: "Space Shuttle", ops: 3, max_roll: 3, vp_first: 4, vp_second: 2, perk: None },
    SpaceBox { name: "Space Station", ops: 4, max_roll: 2, vp_first: 2, vp_second: 0, perk: Some(Perk::EightRounds) },
];

/// Box `n` (1-8).
pub fn space_box(n: u8) -> &'static SpaceBox {
    &TRACK[n as usize - 1]
}

/// `side`'s marker, 0 (start) to 8.
pub fn position(status: &GameStatus, side: Superpower) -> u8 {
    match side {
        Superpower::Us => status.space_race_us,
        Superpower::Ussr => status.space_race_ussr,
    }
}

pub fn set_position(status: &mut GameStatus, side: Superpower, to: u8) {
    match side {
        Superpower::Us => status.space_race_us = to,
        Superpower::Ussr => status.space_race_ussr = to,
    }
}

/// The box `side` would attempt next, or `None` at the end of the track.
pub fn next_box(status: &GameStatus, side: Superpower) -> Option<&'static SpaceBox> {
    let at = position(status, side);
    (at < MAX_BOX).then(|| space_box(at + 1))
}

/// Who currently has `perk`: the side that has reached its box while the
/// opponent hasn't. `None` before anyone arrives and once both have.
pub fn perk_holder(status: &GameStatus, perk: Perk) -> Option<Superpower> {
    let b = perk.box_index();
    let us = position(status, Superpower::Us) >= b;
    let ussr = position(status, Superpower::Ussr) >= b;
    match (us, ussr) {
        (true, false) => Some(Superpower::Us),
        (false, true) => Some(Superpower::Ussr),
        _ => None,
    }
}

/// Space attempts already made this turn.
pub fn attempts_used(status: &GameStatus, side: Superpower) -> u8 {
    match side {
        Superpower::Us => status.space_attempts_us,
        Superpower::Ussr => status.space_attempts_ussr,
    }
}

/// How many attempts `side` may make per turn.
pub fn attempts_allowed(status: &GameStatus, side: Superpower) -> u8 {
    if perk_holder(status, Perk::TwoAttempts) == Some(side) { 2 } else { 1 }
}

/// The VP (in [`GameStatus::vp`]'s signed convention) for `side` arriving
/// at box `n`: the first-in value if the opponent hasn't been there yet,
/// the second-in value if they have.
pub fn arrival_vp(status: &GameStatus, side: Superpower, n: u8) -> i8 {
    let b = space_box(n);
    let vp = if position(status, side.opponent()) >= n { b.vp_second } else { b.vp_first } as i8;
    match side {
        Superpower::Us => vp,
        Superpower::Ussr => -vp,
    }
}

/// Why a card can't be spent on a space attempt right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceError {
    /// A scoring card has no ops.
    Scoring,
    /// The China Card can't be used for the Space Race.
    ChinaCard,
    NotEnoughOps { have: u8, need: u8, target: &'static str },
    NoAttemptsLeft { allowed: u8 },
    TrackComplete,
}

impl fmt::Display for SpaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpaceError::Scoring => write!(f, "a scoring card has no ops to spend on the space race"),
            SpaceError::ChinaCard => write!(f, "the China Card can't be used for the space race"),
            SpaceError::NotEnoughOps { have, need, target } => write!(f, "{target} needs a card of at least {need} ops (this one has {have})"),
            SpaceError::NoAttemptsLeft { allowed: 1 } => write!(f, "already made this turn's space attempt"),
            SpaceError::NoAttemptsLeft { allowed } => write!(f, "already made this turn's {allowed} space attempts"),
            SpaceError::TrackComplete => write!(f, "already at the end of the space race track"),
        }
    }
}

impl std::error::Error for SpaceError {}

/// Whether `side` may spend a card on a space attempt: `ops` is the card's
/// effective ops (after Containment and the like).
pub fn check(status: &GameStatus, side: Superpower, china: bool, scoring: bool, ops: u8) -> Result<(), SpaceError> {
    if scoring {
        return Err(SpaceError::Scoring);
    }
    if china {
        return Err(SpaceError::ChinaCard);
    }
    let target = next_box(status, side).ok_or(SpaceError::TrackComplete)?;
    let allowed = attempts_allowed(status, side);
    if attempts_used(status, side) >= allowed {
        return Err(SpaceError::NoAttemptsLeft { allowed });
    }
    if ops < target.ops {
        return Err(SpaceError::NotEnoughOps { have: ops, need: target.ops, target: target.name });
    }
    Ok(())
}

/// One space attempt, resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceResult {
    pub side: Superpower,
    pub card: CardId,
    /// The marker before the attempt; the target box is `from + 1`.
    pub from: u8,
    pub roll: u8,
    pub max_roll: u8,
    pub success: bool,
    /// Signed like [`GameStatus::vp`]; 0 on a failure.
    pub vp_delta: i8,
}

impl SpaceResult {
    pub fn target(&self) -> &'static SpaceBox {
        space_box(self.from + 1)
    }

    pub fn to(&self) -> u8 {
        self.from + self.success as u8
    }
}

/// Resolves an attempt by `side` with die `roll`. Assumes [`check`] passed.
pub fn resolve(status: &GameStatus, side: Superpower, card: CardId, roll: u8) -> SpaceResult {
    let from = position(status, side);
    let target = space_box(from + 1);
    let success = roll <= target.max_roll;
    let vp_delta = if success { arrival_vp(status, side, from + 1) } else { 0 };
    SpaceResult { side, card, from, roll, max_roll: target.max_roll, success, vp_delta }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Superpower::{Us, Ussr};

    fn at(us: u8, ussr: u8) -> GameStatus {
        GameStatus { space_race_us: us, space_race_ussr: ussr, ..GameStatus::default() }
    }

    #[test]
    fn the_track_matches_the_printed_boxes() {
        assert_eq!(TRACK.len(), MAX_BOX as usize);
        assert_eq!((space_box(1).ops, space_box(1).max_roll, space_box(1).vp_first, space_box(1).vp_second), (2, 3, 2, 1));
        assert_eq!((space_box(7).ops, space_box(7).max_roll, space_box(7).vp_first, space_box(7).vp_second), (3, 3, 4, 2));
        assert_eq!((space_box(8).ops, space_box(8).max_roll), (4, 2));
        for perk in Perk::ALL {
            assert_eq!(space_box(perk.box_index()).perk, Some(perk));
        }
    }

    #[test]
    fn a_perk_belongs_to_the_first_there_until_the_opponent_arrives() {
        assert_eq!(perk_holder(&at(1, 1), Perk::TwoAttempts), None);
        assert_eq!(perk_holder(&at(2, 1), Perk::TwoAttempts), Some(Us));
        assert_eq!(perk_holder(&at(3, 2), Perk::TwoAttempts), None, "cancelled once the opponent reaches the box");
        assert_eq!(perk_holder(&at(0, 8), Perk::EightRounds), Some(Ussr));
    }

    #[test]
    fn arrival_vp_pays_first_and_second_and_signs_by_side() {
        assert_eq!(arrival_vp(&at(0, 0), Us, 1), 2);
        assert_eq!(arrival_vp(&at(0, 0), Ussr, 1), -2);
        assert_eq!(arrival_vp(&at(0, 1), Us, 1), 1);
        assert_eq!(arrival_vp(&at(2, 3), Us, 3), 0, "second into Man in Space scores nothing");
        assert_eq!(arrival_vp(&at(6, 7), Us, 7), 2);
    }

    #[test]
    fn resolve_succeeds_at_or_under_the_box_number() {
        let status = at(0, 0);
        let ok = resolve(&status, Us, CardId(1), 3);
        assert!(ok.success && ok.vp_delta == 2 && ok.to() == 1);
        let miss = resolve(&status, Us, CardId(1), 4);
        assert!(!miss.success && miss.vp_delta == 0 && miss.to() == 0);
    }

    #[test]
    fn check_refuses_what_the_rules_refuse() {
        let status = at(0, 0);
        assert_eq!(check(&status, Us, false, true, 0), Err(SpaceError::Scoring));
        assert_eq!(check(&status, Us, true, false, 4), Err(SpaceError::ChinaCard));
        assert!(matches!(check(&status, Us, false, false, 1), Err(SpaceError::NotEnoughOps { need: 2, .. })));
        assert_eq!(check(&status, Us, false, false, 4), Ok(()), "more ops than needed is fine");
        assert_eq!(check(&at(8, 0), Us, false, false, 4), Err(SpaceError::TrackComplete));
        let spent = GameStatus { space_attempts_us: 1, ..at(0, 0) };
        assert_eq!(check(&spent, Us, false, false, 4), Err(SpaceError::NoAttemptsLeft { allowed: 1 }));
    }

    #[test]
    fn the_box_two_leader_gets_a_second_attempt() {
        let status = GameStatus { space_attempts_us: 1, ..at(2, 1) };
        assert_eq!(attempts_allowed(&status, Us), 2);
        assert_eq!(check(&status, Us, false, false, 2), Ok(()));
        let spent = GameStatus { space_attempts_us: 2, ..at(2, 1) };
        assert_eq!(check(&spent, Us, false, false, 2), Err(SpaceError::NoAttemptsLeft { allowed: 2 }));
    }
}
