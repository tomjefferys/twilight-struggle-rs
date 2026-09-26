use crate::country::{CountryId, Superpower};
use crate::map::WorldMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Influence {
    pub us: u8,
    pub ussr: u8,
}

impl Influence {
    fn get(self, superpower: Superpower) -> u8 {
        match superpower {
            Superpower::Us => self.us,
            Superpower::Ussr => self.ussr,
        }
    }

    fn get_mut(&mut self, superpower: Superpower) -> &mut u8 {
        match superpower {
            Superpower::Us => &mut self.us,
            Superpower::Ussr => &mut self.ussr,
        }
    }
}

/// The mutable game state: how much influence each superpower has in each
/// country. Kept separate from [`WorldMap`] so it stays cheap to clone
/// (for undo/AI lookahead) and never needs to duplicate immutable map data.
#[derive(Debug, Clone)]
pub struct Board {
    influence: Vec<Influence>,
}

impl Board {
    /// A board with zero influence everywhere, sized to match `map`.
    pub fn new(map: &WorldMap) -> Self {
        Board {
            influence: vec![Influence::default(); map.len()],
        }
    }

    pub fn influence(&self, id: CountryId, superpower: Superpower) -> u8 {
        self.influence[id.index()].get(superpower)
    }

    pub fn set_influence(&mut self, id: CountryId, superpower: Superpower, value: u8) {
        *self.influence[id.index()].get_mut(superpower) = value;
    }

    pub fn add_influence(&mut self, id: CountryId, superpower: Superpower, amount: u8) {
        let slot = self.influence[id.index()].get_mut(superpower);
        *slot = slot.saturating_add(amount);
    }

    pub fn remove_influence(&mut self, id: CountryId, superpower: Superpower, amount: u8) {
        let slot = self.influence[id.index()].get_mut(superpower);
        *slot = slot.saturating_sub(amount);
    }

    /// The superpower that controls `id`, if either does: a superpower
    /// controls a country when its influence there is at least the
    /// opponent's influence plus the country's stability.
    pub fn controller(&self, map: &WorldMap, id: CountryId) -> Option<Superpower> {
        let stability = map.country(id).stability;
        let inf = self.influence[id.index()];
        if inf.us >= inf.ussr + stability {
            Some(Superpower::Us)
        } else if inf.ussr >= inf.us + stability {
            Some(Superpower::Ussr)
        } else {
            None
        }
    }

    pub fn is_controlled_by(&self, map: &WorldMap, id: CountryId, superpower: Superpower) -> bool {
        self.controller(map, id) == Some(superpower)
    }
}
