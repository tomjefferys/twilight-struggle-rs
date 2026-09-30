use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use crate::board::Board;
use crate::cards::{CardCatalog, Hands, CHINA_CARD};
use crate::country::Superpower;
use crate::map::WorldMap;
use crate::status::GameStatus;

const DEMO_STATE_JSON: &str = include_str!("../data/demo_state.json");

#[derive(Debug)]
pub enum ScenarioError {
    Json(serde_json::Error),
    UnknownCountry(String),
    UnknownCard(String),
    DuplicateCard(String),
    ChinaCardInHand,
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScenarioError::Json(e) => write!(f, "invalid scenario JSON: {e}"),
            ScenarioError::UnknownCountry(name) => {
                write!(f, "scenario names {name:?}, which is not a country on this map")
            }
            ScenarioError::UnknownCard(name) => {
                write!(f, "scenario names {name:?}, which is not a card in the catalog")
            }
            ScenarioError::DuplicateCard(name) => {
                write!(f, "scenario deals {name:?} to more than one hand")
            }
            ScenarioError::ChinaCardInHand => {
                write!(f, "The China Card can't be dealt into a hand — it's tracked via GameStatus::china_card instead")
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

#[derive(Debug, Deserialize, Default)]
struct RawHands {
    #[serde(default)]
    us: Vec<String>,
    #[serde(default)]
    ussr: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawScenario {
    #[serde(default)]
    status: GameStatus,
    #[serde(default)]
    influence: HashMap<String, [u8; 2]>,
    #[serde(default)]
    hands: RawHands,
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
    pub hands: Hands,
}

impl Scenario {
    /// A hand-authored mid-game position, embedded in the binary, used to
    /// exercise the display views without needing game rules yet.
    pub fn demo(map: &WorldMap, cards: &CardCatalog) -> Result<Self, ScenarioError> {
        Self::from_json(map, cards, DEMO_STATE_JSON)
    }

    pub fn from_json(map: &WorldMap, cards: &CardCatalog, json: &str) -> Result<Self, ScenarioError> {
        let raw: RawScenario = serde_json::from_str(json)?;
        let mut board = Board::new(map);
        for (name, [us, ussr]) in &raw.influence {
            let id = map
                .id_by_name(name)
                .ok_or_else(|| ScenarioError::UnknownCountry(name.clone()))?;
            board.set_influence(id, Superpower::Us, *us);
            board.set_influence(id, Superpower::Ussr, *ussr);
        }

        let mut dealt = std::collections::HashSet::new();
        let resolve_hand = |names: &[String], dealt: &mut std::collections::HashSet<String>| -> Result<Vec<_>, ScenarioError> {
            names
                .iter()
                .map(|name| {
                    let id = cards.id_by_name(name).ok_or_else(|| ScenarioError::UnknownCard(name.clone()))?;
                    if id == CHINA_CARD {
                        return Err(ScenarioError::ChinaCardInHand);
                    }
                    if !dealt.insert(name.clone()) {
                        return Err(ScenarioError::DuplicateCard(name.clone()));
                    }
                    Ok(id)
                })
                .collect()
        };
        let us_hand = resolve_hand(&raw.hands.us, &mut dealt)?;
        let ussr_hand = resolve_hand(&raw.hands.ussr, &mut dealt)?;

        Ok(Scenario { status: raw.status, board, hands: Hands::new(us_hand, ussr_hand) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> (WorldMap, CardCatalog) {
        (WorldMap::standard().unwrap(), CardCatalog::standard().unwrap())
    }

    #[test]
    fn the_demo_scenario_loads_its_hands() {
        let (map, cards) = fixtures();
        let scenario = Scenario::demo(&map, &cards).unwrap();
        assert_eq!(scenario.hands.hand(Superpower::Us).len(), 9);
        assert_eq!(scenario.hands.hand(Superpower::Ussr).len(), 9);
    }

    #[test]
    fn an_unknown_card_name_is_rejected() {
        let (map, cards) = fixtures();
        let json = r#"{"hands": {"us": ["Not A Real Card"]}}"#;
        assert!(matches!(Scenario::from_json(&map, &cards, json), Err(ScenarioError::UnknownCard(_))));
    }

    #[test]
    fn the_same_card_can_not_be_dealt_twice() {
        let (map, cards) = fixtures();
        let json = r#"{"hands": {"us": ["Duck and Cover"], "ussr": ["Duck and Cover"]}}"#;
        assert!(matches!(Scenario::from_json(&map, &cards, json), Err(ScenarioError::DuplicateCard(_))));
    }

    #[test]
    fn the_china_card_can_not_be_dealt_into_a_hand() {
        let (map, cards) = fixtures();
        let json = r#"{"hands": {"ussr": ["The China Card"]}}"#;
        assert!(matches!(Scenario::from_json(&map, &cards, json), Err(ScenarioError::ChinaCardInHand)));
    }
}
