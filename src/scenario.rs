use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::board::Board;
use crate::cards::{CardCatalog, CardId, Hands, CHINA_CARD, MAX_HAND_SIZE};
use crate::country::Superpower;
use crate::map::WorldMap;
use crate::status::{GameStatus, StatusError};

const DEMO_STATE_JSON: &str = include_str!("../data/demo_state.json");

#[derive(Debug)]
pub enum ScenarioError {
    Json(serde_json::Error),
    UnknownCountry(String),
    UnknownCard(String),
    DuplicateCard(String),
    ChinaCardInHand,
    /// The status block itself isn't sane (VP/DEFCON/turn/action round
    /// out of range) — see [`GameStatus::validate`].
    InvalidStatus(StatusError),
    /// `side`'s hand already has more than [`MAX_HAND_SIZE`] cards in it.
    HandTooLarge { side: Superpower, size: usize },
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
                write!(f, "scenario deals {name:?} to more than one hand/discard/removed pile")
            }
            ScenarioError::ChinaCardInHand => {
                write!(
                    f,
                    "The China Card can't be put in a hand, the discard pile, or the removed pile — \
                     it's tracked via GameStatus::china_card instead"
                )
            }
            ScenarioError::InvalidStatus(e) => write!(f, "invalid status: {e}"),
            ScenarioError::HandTooLarge { side, size } => {
                write!(f, "{side}'s hand has {size} cards, more than the {MAX_HAND_SIZE} this crate ever deals")
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

impl From<StatusError> for ScenarioError {
    fn from(e: StatusError) -> Self {
        ScenarioError::InvalidStatus(e)
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct RawHands {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    us: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ussr: Vec<String>,
}

/// The on-disk shape of a [`Scenario`] — a `demo_state.json`-style board/
/// hands/status snapshot, plus (unlike the original demo scenario) the
/// discard and removed-from-game piles, which a test state saved mid-deck
/// can have cards sitting in already. `pub(crate)` rather than private so
/// [`crate::states::StateLibrary`] can embed one directly (`#[serde(flatten)]`)
/// inside its own named-state records, reusing this exact shape rather
/// than redefining it.
#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct RawScenario {
    #[serde(default, skip_serializing_if = "is_default_status")]
    status: GameStatus,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    influence: HashMap<String, [u8; 2]>,
    #[serde(default, skip_serializing_if = "is_default_hands")]
    hands: RawHands,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    discard: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    removed: Vec<String>,
}

fn is_default_status(status: &GameStatus) -> bool {
    *status == GameStatus::default()
}

fn is_default_hands(hands: &RawHands) -> bool {
    hands.us.is_empty() && hands.ussr.is_empty()
}

/// A snapshot of a game in progress: the non-map state plus a populated
/// [`Board`]. This is what the display needs to show something more
/// interesting than an empty board while there are no rules yet to produce
/// game states of its own; later it's also the natural home for the game's
/// real starting position. [`crate::states::StateLibrary`] is the other
/// consumer now — a saved test state round-trips through exactly this
/// type, via [`Scenario::to_raw`]/[`Scenario::from_raw`].
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

    /// An empty board, default status, and empty hands/discard/removed
    /// piles — `main.rs`'s debug-mode `blank` command, as a base for
    /// building a test state up from nothing rather than editing the demo
    /// scenario down to it. Needs no [`CardCatalog`], unlike every other
    /// constructor here, since there's nothing in it yet to resolve a
    /// card name against.
    pub fn blank(map: &WorldMap) -> Self {
        Scenario { status: GameStatus::default(), board: Board::new(map), hands: Hands::default() }
    }

    pub fn from_json(map: &WorldMap, cards: &CardCatalog, json: &str) -> Result<Self, ScenarioError> {
        let raw: RawScenario = serde_json::from_str(json)?;
        Self::from_raw(map, cards, raw)
    }

    /// The shared body of [`Scenario::from_json`] and
    /// [`crate::states::StateLibrary::load`] — resolving a [`RawScenario`]'s
    /// country and card *names* against `map`/`cards` into the real
    /// [`Board`]/[`Hands`] a [`Scenario`] holds. Checks every pile at
    /// once — a hand, the discard pile, or the removed pile — for a
    /// duplicate card or the China Card, since none of the three may ever
    /// hold it (see [`crate::cards::Hands`]'s own doc); also checks the
    /// status itself ([`GameStatus::validate`]) and that neither hand
    /// exceeds [`MAX_HAND_SIZE`] — the same two bounds `main.rs`'s debug
    /// commands enforce on a live game, so a hand-authored (or `save`d)
    /// test state can't load a "nonsensical" position either.
    pub(crate) fn from_raw(map: &WorldMap, cards: &CardCatalog, raw: RawScenario) -> Result<Self, ScenarioError> {
        raw.status.validate()?;
        let mut board = Board::new(map);
        for (name, [us, ussr]) in &raw.influence {
            let id = map
                .id_by_name(name)
                .ok_or_else(|| ScenarioError::UnknownCountry(name.clone()))?;
            board.set_influence(id, Superpower::Us, *us);
            board.set_influence(id, Superpower::Ussr, *ussr);
        }

        let mut dealt = std::collections::HashSet::new();
        let resolve_pile = |names: &[String], dealt: &mut std::collections::HashSet<String>| -> Result<Vec<CardId>, ScenarioError> {
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
        let us_hand = resolve_pile(&raw.hands.us, &mut dealt)?;
        let ussr_hand = resolve_pile(&raw.hands.ussr, &mut dealt)?;
        let discard = resolve_pile(&raw.discard, &mut dealt)?;
        let removed = resolve_pile(&raw.removed, &mut dealt)?;
        if us_hand.len() > MAX_HAND_SIZE {
            return Err(ScenarioError::HandTooLarge { side: Superpower::Us, size: us_hand.len() });
        }
        if ussr_hand.len() > MAX_HAND_SIZE {
            return Err(ScenarioError::HandTooLarge { side: Superpower::Ussr, size: ussr_hand.len() });
        }

        Ok(Scenario { status: raw.status, board, hands: Hands::with_piles(us_hand, ussr_hand, discard, removed) })
    }

    /// The inverse of [`Scenario::from_raw`]: turns this scenario's board/
    /// hands back into name-keyed JSON-shaped data, for
    /// [`crate::states::StateLibrary::save`] to embed. Influence is only
    /// written for a country that actually has any (in either side,
    /// matching how `demo_state.json` is hand-authored), in `map`'s own
    /// load order — not every one of the map's 84 countries, which would
    /// make every saved state file enormous and mostly zeroes.
    pub(crate) fn to_raw(&self, map: &WorldMap, cards: &CardCatalog) -> RawScenario {
        let mut influence = HashMap::new();
        for (id, country) in map.iter() {
            let us = self.board.influence(id, Superpower::Us);
            let ussr = self.board.influence(id, Superpower::Ussr);
            if us != 0 || ussr != 0 {
                influence.insert(country.name.clone(), [us, ussr]);
            }
        }

        let names = |ids: &[CardId]| -> Vec<String> { ids.iter().map(|&id| cards.card(id).name.clone()).collect() };

        RawScenario {
            status: self.status,
            influence,
            hands: RawHands { us: names(self.hands.hand(Superpower::Us)), ussr: names(self.hands.hand(Superpower::Ussr)) },
            discard: names(self.hands.discards()),
            removed: names(self.hands.removed()),
        }
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
    fn a_status_outside_the_valid_ranges_is_rejected() {
        let (map, cards) = fixtures();
        let json = r#"{"status": {"vp": 21}}"#;
        assert!(matches!(Scenario::from_json(&map, &cards, json), Err(ScenarioError::InvalidStatus(_))));
    }

    #[test]
    fn a_hand_bigger_than_max_hand_size_is_rejected() {
        let (map, cards) = fixtures();
        let names: Vec<String> = cards.iter().filter(|c| !c.scoring && c.id != CHINA_CARD).take(MAX_HAND_SIZE + 1).map(|c| format!("{:?}", c.name)).collect();
        let json = format!(r#"{{"hands": {{"us": [{}]}}}}"#, names.join(", "));
        assert!(matches!(Scenario::from_json(&map, &cards, &json), Err(ScenarioError::HandTooLarge { side: Superpower::Us, .. })));
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
    fn the_same_card_can_not_be_in_a_hand_and_the_discard_pile() {
        let (map, cards) = fixtures();
        let json = r#"{"hands": {"us": ["Duck and Cover"]}, "discard": ["Duck and Cover"]}"#;
        assert!(matches!(Scenario::from_json(&map, &cards, json), Err(ScenarioError::DuplicateCard(_))));
    }

    #[test]
    fn the_china_card_can_not_be_dealt_into_a_hand() {
        let (map, cards) = fixtures();
        let json = r#"{"hands": {"ussr": ["The China Card"]}}"#;
        assert!(matches!(Scenario::from_json(&map, &cards, json), Err(ScenarioError::ChinaCardInHand)));
    }

    #[test]
    fn the_china_card_can_not_be_in_the_discard_pile() {
        let (map, cards) = fixtures();
        let json = r#"{"discard": ["The China Card"]}"#;
        assert!(matches!(Scenario::from_json(&map, &cards, json), Err(ScenarioError::ChinaCardInHand)));
    }

    #[test]
    fn the_china_card_can_not_be_in_the_removed_pile() {
        let (map, cards) = fixtures();
        let json = r#"{"removed": ["The China Card"]}"#;
        assert!(matches!(Scenario::from_json(&map, &cards, json), Err(ScenarioError::ChinaCardInHand)));
    }

    #[test]
    fn to_raw_round_trips_through_from_raw() {
        let (map, cards) = fixtures();
        let scenario = Scenario::demo(&map, &cards).unwrap();
        let raw = scenario.to_raw(&map, &cards);
        let round_tripped = Scenario::from_raw(&map, &cards, raw).unwrap();
        assert_eq!(round_tripped.status, scenario.status);
        assert_eq!(round_tripped.hands.hand(Superpower::Us), scenario.hands.hand(Superpower::Us));
        assert_eq!(round_tripped.hands.hand(Superpower::Ussr), scenario.hands.hand(Superpower::Ussr));
        for (id, _) in map.iter() {
            assert_eq!(round_tripped.board.influence(id, Superpower::Us), scenario.board.influence(id, Superpower::Us));
            assert_eq!(round_tripped.board.influence(id, Superpower::Ussr), scenario.board.influence(id, Superpower::Ussr));
        }
    }
}
