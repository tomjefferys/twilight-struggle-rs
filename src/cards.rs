//! The game's cards: the static catalog ([`CardCatalog`], loaded from
//! `data/cards.json`) and each side's [`Hands`] of held cards. Mirrors
//! `map.rs`'s own split — immutable, validated-at-load data
//! (`CardCatalog`) kept separate from the mutable per-game state
//! (`Hands`, alongside `Board` in spirit) — though `Hands` lives here
//! rather than in its own module since it's a thin `[Vec<CardId>; 2]`
//! with nothing else to say about it. No card *behaviour* yet: nothing in
//! this module plays, draws, or discards a card — that's future work once
//! the display and UI around a hand are settled.

use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use crate::country::Superpower;

const CARDS_JSON: &str = include_str!("../data/cards.json");

/// The id of the China Card — the one card that never lives in a
/// [`Hands`] list (it's tracked separately, via `GameStatus::china_card`),
/// since it changes hands outside the normal draw/discard cycle and is
/// always shown as its own slot rather than sorted into a hand.
pub const CHINA_CARD: CardId = CardId(6);

/// Index of a [`Card`] within a [`CardCatalog`].
///
/// Unlike [`crate::country::CountryId`], this carries the *real* Twilight
/// Struggle card number (1-110) rather than a zero-based load-order index
/// — that number is printed on the physical card and shown on screen
/// (`card #31`), so keeping it as the id avoids a separate "card number"
/// field that could drift from the index used to look it up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize)]
pub struct CardId(pub(crate) u8);

impl CardId {
    pub fn number(self) -> u8 {
        self.0
    }
}

impl fmt::Display for CardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Which side(s) a card belongs to — whose hand it's dealt into, and (for
/// display) which colour it's tinted. A scoring card or an event playable
/// by either side is [`CardSide::Neutral`], the odd one out among an
/// otherwise US/USSR-only crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum CardSide {
    #[serde(rename = "US")]
    Us,
    #[serde(rename = "USSR")]
    Ussr,
    #[serde(rename = "Both")]
    Neutral,
}

impl fmt::Display for CardSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CardSide::Us => "US",
            CardSide::Ussr => "USSR",
            CardSide::Neutral => "Both",
        };
        write!(f, "{s}")
    }
}

/// The three broad eras a card belongs to, matching the physical deck's
/// own Early/Mid/Late War split (and its yellow/brown/red card backs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum CardPhase {
    Early,
    Mid,
    Late,
}

impl fmt::Display for CardPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CardPhase::Early => "Early War",
            CardPhase::Mid => "Mid War",
            CardPhase::Late => "Late War",
        };
        write!(f, "{s}")
    }
}

impl CardPhase {
    /// Just "Early"/"Mid"/"Late" — the hand strip's own mini-card slots
    /// have no room for "War" on every one of them.
    pub fn short(self) -> &'static str {
        match self {
            CardPhase::Early => "Early",
            CardPhase::Mid => "Mid",
            CardPhase::Late => "Late",
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawCard {
    id: u8,
    name: String,
    ops: u8,
    phase: CardPhase,
    side: CardSide,
    optional: bool,
    removed_after_event: bool,
    ongoing: bool,
    scoring: bool,
    text: String,
}

#[derive(Debug, Deserialize)]
struct RawCatalog {
    cards: Vec<RawCard>,
}

/// One card's full data: everything shown on the hand strip and the
/// zoomed-in detail view. Nothing here says what playing it *does* — that
/// behaviour doesn't exist yet.
#[derive(Debug, Clone)]
pub struct Card {
    pub id: CardId,
    pub name: String,
    /// Operations value, 0 for a scoring card (it's never spent as ops).
    pub ops: u8,
    pub phase: CardPhase,
    pub side: CardSide,
    /// Optional cards may be discarded without being played; required
    /// ones must eventually be played or discarded through the normal
    /// hand-size rules (rule 3.2 — not enforced yet, just recorded).
    pub optional: bool,
    /// Removed from the game after being played as an event ("Trash" on
    /// the physical card's edge) — most one-shot events.
    pub removed_after_event: bool,
    /// Its event has a lasting effect on the game rather than a one-time
    /// one ("Permanent" on the card).
    pub ongoing: bool,
    /// A scoring card (`ops == 0`, may not be held past the end of the
    /// turn it's drawn — rule 3.2.1).
    pub scoring: bool,
    pub text: String,
}

impl Card {
    /// How this card's ops value is shown wherever space is tight (the
    /// hand strip's mini-card boxes, the zoom view's own title) — its
    /// digit, `S` for a scoring card (whose `ops` is always 0 and never
    /// spent as ops), or `★{ops}` for the China Card, to set it apart
    /// from an ordinary card of the same value.
    pub fn ops_label(&self) -> String {
        if self.id == CHINA_CARD {
            format!("★{}", self.ops)
        } else if self.scoring {
            "S".to_string()
        } else {
            self.ops.to_string()
        }
    }
}

#[derive(Debug)]
pub enum CardError {
    Json(serde_json::Error),
    DuplicateId(u8),
    DuplicateName(String),
    NonContiguousIds,
    ScoringOpsMismatch { id: u8, ops: u8 },
}

impl fmt::Display for CardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CardError::Json(e) => write!(f, "invalid card JSON: {e}"),
            CardError::DuplicateId(id) => write!(f, "duplicate card id: {id}"),
            CardError::DuplicateName(name) => write!(f, "duplicate card name: {name}"),
            CardError::NonContiguousIds => write!(f, "card ids are not a contiguous 1..=N range"),
            CardError::ScoringOpsMismatch { id, ops } => {
                write!(f, "card {id} is marked scoring but has ops value {ops} (expected 0)")
            }
        }
    }
}

impl std::error::Error for CardError {}

impl From<serde_json::Error> for CardError {
    fn from(e: serde_json::Error) -> Self {
        CardError::Json(e)
    }
}

/// The result of a forgiving, interactive card lookup ([`CardCatalog::find`]),
/// the card analogue of [`crate::map::Found`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardFound {
    One(CardId),
    Ambiguous(Vec<CardId>),
    None,
}

/// The full set of cards for one edition of the game (currently just the
/// standard deck). Immutable once loaded and validated — every lookup
/// through this type is infallible.
#[derive(Debug)]
pub struct CardCatalog {
    cards: Vec<Card>,
    by_id: HashMap<CardId, usize>,
}

impl CardCatalog {
    /// The standard Twilight Struggle card deck, embedded in the binary —
    /// converted from the game's own card CSV; see `CLAUDE.md`'s
    /// `data/cards.json` entry for the conversion.
    pub fn standard() -> Result<Self, CardError> {
        Self::from_json(CARDS_JSON)
    }

    pub fn from_json(json: &str) -> Result<Self, CardError> {
        let raw: RawCatalog = serde_json::from_str(json)?;

        let mut by_id = HashMap::with_capacity(raw.cards.len());
        let mut by_name = std::collections::HashSet::with_capacity(raw.cards.len());
        let mut cards = Vec::with_capacity(raw.cards.len());
        for entry in raw.cards {
            let id = CardId(entry.id);
            if by_id.insert(id, cards.len()).is_some() {
                return Err(CardError::DuplicateId(entry.id));
            }
            if !by_name.insert(entry.name.clone()) {
                return Err(CardError::DuplicateName(entry.name));
            }
            if entry.scoring != (entry.ops == 0) {
                return Err(CardError::ScoringOpsMismatch { id: entry.id, ops: entry.ops });
            }
            cards.push(Card {
                id,
                name: entry.name,
                ops: entry.ops,
                phase: entry.phase,
                side: entry.side,
                optional: entry.optional,
                removed_after_event: entry.removed_after_event,
                ongoing: entry.ongoing,
                scoring: entry.scoring,
                text: entry.text,
            });
        }

        let mut ids: Vec<u8> = by_id.keys().map(|id| id.0).collect();
        ids.sort_unstable();
        if ids != (1..=ids.len() as u8).collect::<Vec<u8>>() {
            return Err(CardError::NonContiguousIds);
        }

        Ok(CardCatalog { cards, by_id })
    }

    pub fn card(&self, id: CardId) -> &Card {
        &self.cards[self.by_id[&id]]
    }

    pub fn id_by_name(&self, name: &str) -> Option<CardId> {
        self.cards.iter().find(|c| c.name == name).map(|c| c.id)
    }

    /// A forgiving name-or-number lookup for interactive use, the card
    /// analogue of [`crate::map::WorldMap::find`]: a numeric query matches
    /// that card's id exactly; otherwise an exact name match (any case)
    /// wins outright, else every card whose name starts with `query` (any
    /// case) is a candidate.
    pub fn find(&self, query: &str) -> CardFound {
        if let Ok(number) = query.parse::<u8>() {
            return match self.by_id.get(&CardId(number)) {
                Some(_) => CardFound::One(CardId(number)),
                None => CardFound::None,
            };
        }
        let query_lower = query.to_lowercase();
        for card in &self.cards {
            if card.name.to_lowercase() == query_lower {
                return CardFound::One(card.id);
            }
        }
        let matches: Vec<CardId> = self
            .cards
            .iter()
            .filter(|c| c.name.to_lowercase().starts_with(&query_lower))
            .map(|c| c.id)
            .collect();
        match matches.len() {
            0 => CardFound::None,
            1 => CardFound::One(matches[0]),
            _ => CardFound::Ambiguous(matches),
        }
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }
}

/// Each side's held cards — a hand is just the list of [`CardId`]s dealt
/// to it, in hand order. Deliberately doesn't include the China Card
/// ([`CHINA_CARD`]), which is tracked separately (`GameStatus::china_card`)
/// since it changes hands outside the normal draw/discard cycle.
///
/// Cheap to clone, like [`crate::board::Board`] and every other piece of
/// per-game state `Game` carries — the same reason: `Game` itself needs to
/// stay clonable for AI lookahead.
#[derive(Debug, Clone, Default)]
pub struct Hands {
    us: Vec<CardId>,
    ussr: Vec<CardId>,
}

impl Hands {
    pub fn new(us: Vec<CardId>, ussr: Vec<CardId>) -> Self {
        Hands { us, ussr }
    }

    pub fn hand(&self, side: Superpower) -> &[CardId] {
        match side {
            Superpower::Us => &self.us,
            Superpower::Ussr => &self.ussr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_catalog_loads_all_110_cards() {
        let catalog = CardCatalog::standard().unwrap();
        assert_eq!(catalog.len(), 110);
    }

    #[test]
    fn the_china_card_is_present_and_named() {
        let catalog = CardCatalog::standard().unwrap();
        assert_eq!(catalog.card(CHINA_CARD).name, "The China Card");
    }

    #[test]
    fn every_card_has_a_unique_id_and_name() {
        let catalog = CardCatalog::standard().unwrap();
        let mut ids = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for id in 1..=catalog.len() as u8 {
            let card = catalog.card(CardId(id));
            assert!(ids.insert(card.id), "duplicate id {id}");
            assert!(names.insert(card.name.clone()), "duplicate name {}", card.name);
        }
    }

    #[test]
    fn every_scoring_card_has_zero_ops_and_vice_versa() {
        let catalog = CardCatalog::standard().unwrap();
        for id in 1..=catalog.len() as u8 {
            let card = catalog.card(CardId(id));
            assert_eq!(card.scoring, card.ops == 0, "card {id} ({}) scoring/ops mismatch", card.name);
        }
    }

    #[test]
    fn find_matches_by_number_exact_name_and_prefix() {
        let catalog = CardCatalog::standard().unwrap();
        assert_eq!(catalog.find("6"), CardFound::One(CHINA_CARD));
        assert_eq!(catalog.find("the china card"), CardFound::One(CHINA_CARD));
        assert_eq!(catalog.find("nonexistent card name"), CardFound::None);
        assert_eq!(catalog.find("999"), CardFound::None);
    }

    #[test]
    fn find_reports_ambiguous_prefixes() {
        let catalog = CardCatalog::standard().unwrap();
        // Both "Nuclear Test Ban" and "Nuclear Subs" start with "Nuclear".
        match catalog.find("Nuclear") {
            CardFound::Ambiguous(matches) => assert!(matches.len() >= 2),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn a_duplicate_id_is_rejected() {
        let json = r#"{"cards": [
            {"id": 1, "name": "A", "ops": 1, "phase": "Early", "side": "US", "optional": false, "removed_after_event": false, "ongoing": false, "scoring": false, "text": "x"},
            {"id": 1, "name": "B", "ops": 1, "phase": "Early", "side": "US", "optional": false, "removed_after_event": false, "ongoing": false, "scoring": false, "text": "x"}
        ]}"#;
        assert!(matches!(CardCatalog::from_json(json), Err(CardError::DuplicateId(1))));
    }

    #[test]
    fn a_scoring_ops_mismatch_is_rejected() {
        let json = r#"{"cards": [
            {"id": 1, "name": "A", "ops": 2, "phase": "Early", "side": "US", "optional": false, "removed_after_event": false, "ongoing": false, "scoring": true, "text": "x"}
        ]}"#;
        assert!(matches!(CardCatalog::from_json(json), Err(CardError::ScoringOpsMismatch { id: 1, ops: 2 })));
    }

    #[test]
    fn non_contiguous_ids_are_rejected() {
        let json = r#"{"cards": [
            {"id": 1, "name": "A", "ops": 1, "phase": "Early", "side": "US", "optional": false, "removed_after_event": false, "ongoing": false, "scoring": false, "text": "x"},
            {"id": 3, "name": "B", "ops": 1, "phase": "Early", "side": "US", "optional": false, "removed_after_event": false, "ongoing": false, "scoring": false, "text": "x"}
        ]}"#;
        assert!(matches!(CardCatalog::from_json(json), Err(CardError::NonContiguousIds)));
    }
}
