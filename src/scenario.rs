use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use crate::board::Board;
use crate::country::Superpower;
use crate::map::WorldMap;
use crate::status::GameStatus;

const DEMO_STATE_JSON: &str = include_str!("../data/demo_state.json");

#[derive(Debug)]
pub enum ScenarioError {
    Json(serde_json::Error),
    UnknownCountry(String),
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScenarioError::Json(e) => write!(f, "invalid scenario JSON: {e}"),
            ScenarioError::UnknownCountry(name) => {
                write!(f, "scenario names {name:?}, which is not a country on this map")
            }
        }
    }
}

impl std::error::Error for ScenarioError {}

impl From<serde_json::Error> for ScenarioError {
    fn from(e: serde_json::Error) -> Self {
        ScenarioError::Json(e)
    }
}

#[derive(Debug, Deserialize)]
struct RawScenario {
    #[serde(default)]
    status: GameStatus,
    #[serde(default)]
    influence: HashMap<String, [u8; 2]>,
}

/// A snapshot of a game in progress: the non-map state plus a populated
/// [`Board`]. This is what the display needs to show something more
/// interesting than an empty board while there are no rules yet to produce
/// game states of its own; later it's also the natural home for the game's
/// real starting position.
#[derive(Debug, Clone)]
pub struct Scenario {
    pub status: GameStatus,
    pub board: Board,
}

impl Scenario {
    /// A hand-authored mid-game position, embedded in the binary, used to
    /// exercise the display views without needing game rules yet.
    pub fn demo(map: &WorldMap) -> Result<Self, ScenarioError> {
        Self::from_json(map, DEMO_STATE_JSON)
    }

    pub fn from_json(map: &WorldMap, json: &str) -> Result<Self, ScenarioError> {
        let raw: RawScenario = serde_json::from_str(json)?;
        let mut board = Board::new(map);
        for (name, [us, ussr]) in &raw.influence {
            let id = map
                .id_by_name(name)
                .ok_or_else(|| ScenarioError::UnknownCountry(name.clone()))?;
            board.set_influence(id, Superpower::Us, *us);
            board.set_influence(id, Superpower::Ussr, *ussr);
        }
        Ok(Scenario { status: raw.status, board })
    }
}
