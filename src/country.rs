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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
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

/// An arrow-key direction, for navigating between regions on the world map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Region {
    /// All six regions, in no particular order.
    pub const ALL: [Region; 6] = [
        Region::Europe,
        Region::Asia,
        Region::MiddleEast,
        Region::Africa,
        Region::CentralAmerica,
        Region::SouthAmerica,
    ];

    /// The region reached by moving `dir` from this one on the world map,
    /// or `None` if there isn't a sensible neighbour that way — the
    /// selection should then hold still rather than wrap around.
    ///
    /// Hand-tuned against each region's real `world_cell` positions in
    /// `data/standard_layout.json` (mean row/col: Europe 8/89, Asia 17/135,
    /// Middle East 15/103, Africa 21/94, Central America 18/45, South
    /// America 26/57), rather than derived from them, so a move always
    /// lands somewhere predictable.
    pub fn step(self, dir: Direction) -> Option<Region> {
        use Direction::*;
        use Region::*;
        Some(match (self, dir) {
            (Europe, Left) => CentralAmerica,
            (Europe, Right) => Asia,
            (Europe, Down) => MiddleEast,

            (Asia, Left) => MiddleEast,
            (Asia, Up) => Europe,

            (MiddleEast, Left) => Africa,
            (MiddleEast, Right) => Asia,
            (MiddleEast, Up) => Europe,
            (MiddleEast, Down) => Africa,

            (Africa, Left) => SouthAmerica,
            (Africa, Right) => MiddleEast,
            (Africa, Up) => Europe,

            (CentralAmerica, Right) => Africa,
            (CentralAmerica, Up) => Europe,
            (CentralAmerica, Down) => SouthAmerica,

            (SouthAmerica, Right) => Africa,
            (SouthAmerica, Up) => CentralAmerica,

            _ => return None,
        })
    }
}

#[cfg(test)]
mod region_navigation_tests {
    use super::*;

    #[test]
    fn every_region_is_reachable_from_europe() {
        // Breadth-first search over `step` from Europe should reach all
        // six regions — otherwise some region would be an unreachable
        // island on the world map.
        let mut seen = vec![Region::Europe];
        let mut frontier = vec![Region::Europe];
        while let Some(region) = frontier.pop() {
            for dir in [Direction::Up, Direction::Down, Direction::Left, Direction::Right] {
                if let Some(next) = region.step(dir)
                    && !seen.contains(&next)
                {
                    seen.push(next);
                    frontier.push(next);
                }
            }
        }
        for &region in &Region::ALL {
            assert!(seen.contains(&region), "{region} is unreachable from Europe");
        }
    }

    #[test]
    fn step_never_returns_the_starting_region() {
        for &region in &Region::ALL {
            for dir in [Direction::Up, Direction::Down, Direction::Left, Direction::Right] {
                assert_ne!(region.step(dir), Some(region), "{region} stepping {dir:?} returned itself");
            }
        }
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

impl fmt::Display for SubRegion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            SubRegion::WesternEurope => "Western Europe",
            SubRegion::EasternEurope => "Eastern Europe",
            SubRegion::SoutheastAsia => "Southeast Asia",
        };
        write!(f, "{s}")
    }
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
