use std::fmt;

use serde::{Deserialize, Serialize};

use crate::country::Superpower;
use crate::ongoing::TurnEffects;

/// The VP track's own cap (rule 5.5) — reaching either end wins the game
/// outright ([`crate::game::Game::apply_vp`]), so a valid [`GameStatus`]
/// never sits outside it.
pub const VP_RANGE: std::ops::RangeInclusive<i8> = -20..=20;

/// DEFCON only ever has five levels. DEFCON degradation itself (rule
/// 7.? / Military Operations) is out of scope for this crate — nothing
/// here ever *moves* the track — but the five levels are the track
/// itself, not part of that unimplemented rule.
pub const DEFCON_RANGE: std::ops::RangeInclusive<u8> = 1..=5;

/// The real game runs exactly ten turns.
pub const TURN_RANGE: std::ops::RangeInclusive<u8> = 1..=10;

/// The real game deals at most seven action rounds in a turn (six for
/// turns 1-3, seven from turn 4 on — a distinction this crate doesn't
/// derive automatically, see `GameStatus::action_rounds_per_turn`'s own
/// field, so [`GameStatus::validate`] only checks the number is
/// *plausible*, not that it matches `turn`).
pub const ACTION_ROUNDS_PER_TURN_RANGE: std::ops::RangeInclusive<u8> = 1..=7;

/// Why a [`GameStatus`] isn't one [`GameStatus::validate`] accepts —
/// surfaced by a hand-authored `Scenario`/test-state JSON file that fails
/// to load ([`crate::scenario::ScenarioError::InvalidStatus`]) and by
/// `main.rs`'s debug-mode `vp`/`defcon`/`turn`/`ar` commands, which both
/// go through this one check rather than duplicating the ranges above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusError {
    VpOutOfRange(i8),
    DefconOutOfRange(u8),
    TurnOutOfRange(u8),
    ActionRoundsPerTurnOutOfRange(u8),
    /// `action_round` isn't between 1 and `action_rounds_per_turn` —
    /// named together, since which values are valid depends on the
    /// other field.
    ActionRoundOutOfRange { action_round: u8, action_rounds_per_turn: u8 },
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StatusError::VpOutOfRange(n) => write!(f, "vp {n} is outside the track's own range ({}..={})", VP_RANGE.start(), VP_RANGE.end()),
            StatusError::DefconOutOfRange(n) => {
                write!(f, "defcon {n} is outside the valid range ({}..={})", DEFCON_RANGE.start(), DEFCON_RANGE.end())
            }
            StatusError::TurnOutOfRange(n) => {
                write!(f, "turn {n} is outside the game's own length ({}..={})", TURN_RANGE.start(), TURN_RANGE.end())
            }
            StatusError::ActionRoundsPerTurnOutOfRange(n) => write!(
                f,
                "{n} action rounds in a turn isn't plausible ({}..={})",
                ACTION_ROUNDS_PER_TURN_RANGE.start(),
                ACTION_ROUNDS_PER_TURN_RANGE.end()
            ),
            StatusError::ActionRoundOutOfRange { action_round, action_rounds_per_turn } => write!(
                f,
                "action round {action_round} isn't between 1 and {action_rounds_per_turn} (this turn's own action_rounds_per_turn)"
            ),
        }
    }
}

impl std::error::Error for StatusError {}

/// The parts of the overall game state that aren't per-country influence:
/// turn tracking, DEFCON, victory points, the space race, and the China
/// Card. Plain data with no rules attached — [`crate::game::Game`] is what
/// actually advances `turn`/`action_round`/`active`; this struct just holds
/// the values so the display header can show them.
///
/// `Serialize` (alongside the existing `Deserialize`) is what lets
/// [`crate::states::StateLibrary::save`] write a [`crate::scenario::Scenario`]'s
/// status back out as JSON, the same shape [`crate::scenario::Scenario::from_json`]
/// reads in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameStatus {
    pub turn: u8,
    pub action_round: u8,
    pub action_rounds_per_turn: u8,
    /// The side whose turn it is to act this action round. USSR acts
    /// first in each action round, per the real rules.
    pub active: Superpower,
    pub defcon: u8,
    /// Victory points: positive favours the USA, negative the USSR.
    pub vp: i8,
    pub space_race_us: u8,
    pub space_race_ussr: u8,
    pub military_ops_us: i8,
    pub military_ops_ussr: i8,
    pub china_card: Superpower,
    pub china_card_face_up: bool,
    /// Card events in force until the turn ends (see [`crate::ongoing`]).
    #[serde(skip_serializing_if = "TurnEffects::is_empty")]
    pub effects: TurnEffects,
}

impl GameStatus {
    /// Whether every field is within the bounds a real `GameStatus`
    /// could actually be in — the one check both a loaded `Scenario`/
    /// test state and a live debug-mode edit (`vp`/`defcon`/`turn`/`ar`)
    /// go through, so "nonsensical" means the same thing in both places.
    /// Deliberately doesn't check `space_race_us`/`ussr` or
    /// `military_ops_us`/`ussr` — both tracks belong to rules this crate
    /// doesn't implement at all yet (DEFCON degradation, Military
    /// Operations), so this has no real basis to bound them, unlike
    /// DEFCON's five levels or the VP cap, which are the tracks
    /// themselves rather than a rule about moving them.
    pub fn validate(&self) -> Result<(), StatusError> {
        if !VP_RANGE.contains(&self.vp) {
            return Err(StatusError::VpOutOfRange(self.vp));
        }
        if !DEFCON_RANGE.contains(&self.defcon) {
            return Err(StatusError::DefconOutOfRange(self.defcon));
        }
        if !TURN_RANGE.contains(&self.turn) {
            return Err(StatusError::TurnOutOfRange(self.turn));
        }
        if !ACTION_ROUNDS_PER_TURN_RANGE.contains(&self.action_rounds_per_turn) {
            return Err(StatusError::ActionRoundsPerTurnOutOfRange(self.action_rounds_per_turn));
        }
        // North Sea Oil gives the US one action round past the usual count.
        let extra_round = self.action_round == self.action_rounds_per_turn + 1
            && self.active == Superpower::Us
            && self.effects.extra_rounds(Superpower::Us) > 0;
        if self.action_round < 1 || (self.action_round > self.action_rounds_per_turn && !extra_round) {
            return Err(StatusError::ActionRoundOutOfRange {
                action_round: self.action_round,
                action_rounds_per_turn: self.action_rounds_per_turn,
            });
        }
        Ok(())
    }
}

impl Default for GameStatus {
    fn default() -> Self {
        GameStatus {
            turn: 1,
            action_round: 1,
            action_rounds_per_turn: 6,
            active: Superpower::Ussr,
            defcon: 5,
            vp: 0,
            space_race_us: 0,
            space_race_ussr: 0,
            military_ops_us: 0,
            military_ops_ussr: 0,
            china_card: Superpower::Ussr,
            china_card_face_up: true,
            effects: TurnEffects::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_status_validates() {
        assert_eq!(GameStatus::default().validate(), Ok(()));
    }

    #[test]
    fn vp_outside_the_track_is_rejected() {
        let status = GameStatus { vp: 21, ..GameStatus::default() };
        assert_eq!(status.validate(), Err(StatusError::VpOutOfRange(21)));
        let status = GameStatus { vp: -21, ..GameStatus::default() };
        assert_eq!(status.validate(), Err(StatusError::VpOutOfRange(-21)));
    }

    #[test]
    fn defcon_outside_one_through_five_is_rejected() {
        let status = GameStatus { defcon: 0, ..GameStatus::default() };
        assert_eq!(status.validate(), Err(StatusError::DefconOutOfRange(0)));
        let status = GameStatus { defcon: 6, ..GameStatus::default() };
        assert_eq!(status.validate(), Err(StatusError::DefconOutOfRange(6)));
    }

    #[test]
    fn turn_outside_one_through_ten_is_rejected() {
        let status = GameStatus { turn: 11, ..GameStatus::default() };
        assert_eq!(status.validate(), Err(StatusError::TurnOutOfRange(11)));
    }

    #[test]
    fn action_round_must_fit_within_action_rounds_per_turn() {
        let status = GameStatus { action_round: 7, action_rounds_per_turn: 6, ..GameStatus::default() };
        assert_eq!(status.validate(), Err(StatusError::ActionRoundOutOfRange { action_round: 7, action_rounds_per_turn: 6 }));
        let status = GameStatus { action_round: 0, ..GameStatus::default() };
        assert_eq!(status.validate(), Err(StatusError::ActionRoundOutOfRange { action_round: 0, action_rounds_per_turn: 6 }));
        // The same action_round is fine once action_rounds_per_turn grows
        // to fit it.
        let status = GameStatus { action_round: 7, action_rounds_per_turn: 7, ..GameStatus::default() };
        assert_eq!(status.validate(), Ok(()));
    }
}
