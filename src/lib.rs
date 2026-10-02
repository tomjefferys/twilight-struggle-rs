pub mod action;
pub mod ai;
pub mod board;
pub mod cards;
pub mod country;
pub mod dice;
pub mod events;
pub mod game;
pub mod layout;
pub mod log;
pub mod map;
pub mod ops;
pub mod render;
pub mod scenario;
pub mod states;
pub mod status;

pub use action::Action;
pub use ai::{play_turn, Ai, RandomAi};
pub use board::{Board, Influence};
pub use cards::{Card, CardCatalog, CardError, CardFound, CardId, CardPhase, CardSide, Hands, CHINA_CARD, MAX_HAND_SIZE};
pub use country::{Country, CountryId, Direction, Region, SubRegion, Superpower};
pub use dice::Dice;
pub use events::{scoring, EventOutcome};
pub use game::{Game, GameError, OperationKind, RollOutcome};
pub use layout::{Cell, Guest, GuestEntity, LayoutError, LinkTarget, MapLayout, UndrawnLink};
pub use log::{Event, GameLog, LogEntry};
pub use map::{Found, MapError, WorldMap};
pub use ops::{
    coup_odds, coup_resolve, coup_target_number, modifiers, odds, resolve, Coup, CoupError, CoupOdds, CoupResult, InfluencePlacement,
    Modifiers, Odds, Operation, PlacementError, RealignError, Realignment, RollResult,
};
pub use render::{ColorMode, ViewMode};
pub use scenario::{Scenario, ScenarioError};
pub use states::{StateEntry, StateError, StateLibrary};
pub use status::{GameStatus, StatusError, ACTION_ROUNDS_PER_TURN_RANGE, DEFCON_RANGE, TURN_RANGE, VP_RANGE};
