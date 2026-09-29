use serde::Deserialize;

use crate::country::Superpower;

/// The parts of the overall game state that aren't per-country influence:
/// turn tracking, DEFCON, victory points, the space race, and the China
/// Card. Plain data with no rules attached — [`crate::game::Game`] is what
/// actually advances `turn`/`action_round`/`active`; this struct just holds
/// the values so the display header can show them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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
        }
    }
}
