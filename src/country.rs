use std::fmt;

/// Index of a [`Country`] within a [`crate::map::WorldMap`].
///
/// Only ever constructed by `WorldMap` itself during loading, so a valid
/// `CountryId` is guaranteed to index into the map that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CountryId(pub(crate) u16);

impl CountryId {
    pub(crate) fn new(index: usize) -> Self {
        CountryId(index as u16)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Superpower {
    Us,
    Ussr,
}

impl Superpower {
    pub fn opponent(self) -> Superpower {
        match self {
            Superpower::Us => Superpower::Ussr,
            Superpower::Ussr => Superpower::Us,
        }
    }
}

impl fmt::Display for Superpower {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Superpower::Us => write!(f, "USA"),
            Superpower::Ussr => write!(f, "USSR"),
        }
    }
}

/// The six regions used for control scoring. Every country belongs to
/// exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize)]
pub enum Region {
    Europe,
    Asia,
    MiddleEast,
    Africa,
    CentralAmerica,
    SouthAmerica,
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Region::Europe => "Europe",
            Region::Asia => "Asia",
            Region::MiddleEast => "Middle East",
            Region::Africa => "Africa",
            Region::CentralAmerica => "Central America",
            Region::SouthAmerica => "South America",
        };
        write!(f, "{s}")
    }
}

/// Finer-grained groupings referenced by specific event cards
/// (e.g. "Warsaw Pact Formed" targets Eastern Europe, "SE Asia Scoring"
/// targets Southeast Asia). A country can belong to zero or more of these,
/// independent of its single scoring [`Region`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
pub enum SubRegion {
    WesternEurope,
    EasternEurope,
    SoutheastAsia,
}

/// The immutable properties of a country on the board: its name, how hard
/// it is to control, whether it's fought over by scoring cards, which
/// region(s) it belongs to, and which countries border it.
///
/// This never changes once loaded. The mutable, per-game influence values
/// live separately in [`crate::board::Board`].
#[derive(Debug, Clone)]
pub struct Country {
    pub name: String,
    pub stability: u8,
    pub battleground: bool,
    pub region: Region,
    pub sub_regions: Vec<SubRegion>,
    pub adjacent: Vec<CountryId>,
    pub adjacent_superpowers: Vec<Superpower>,
}

impl Country {
    pub fn is_in_sub_region(&self, sub_region: SubRegion) -> bool {
        self.sub_regions.contains(&sub_region)
    }

    pub fn borders_superpower(&self, superpower: Superpower) -> bool {
        self.adjacent_superpowers.contains(&superpower)
    }
}
