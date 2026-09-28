//! Ops-spending operations: staged actions that spend a superpower's
//! operation points against a [`Board`] before anything reaches the real
//! game state.
//!
//! There are two shapes here, not one. [`InfluencePlacement`] stages
//! every point against a cloned board and only writes to the real one on
//! [`InfluencePlacement::commit`] — so a whole action can be undone or
//! discarded. [`Realignment`] is deliberately different: each roll is
//! resolved immediately onto the real board and — like a roll at a
//! physical table — can't be taken back. See each type's own module doc
//! for why.
//!
//! [`Operation`] is the seam the rest of the crate (`main.rs`,
//! `interactive.rs`, `render/`) uses to treat "whichever operation is
//! open" uniformly, without caring which kind it is.

mod influence;
mod realign;

pub use influence::{InfluencePlacement, PlacementError};
pub use realign::{modifiers, odds, resolve, Modifiers, Odds, RealignError, Realignment, RollResult};

use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::map::WorldMap;

/// Whichever ops-spending operation is currently open, if any — the
/// shared handle every view and every REPL command reads through rather
/// than matching on the concrete type itself.
pub enum Operation {
    Influence(InfluencePlacement),
    Realign(Realignment),
}

impl Operation {
    pub fn side(&self) -> Superpower {
        match self {
            Operation::Influence(p) => p.side(),
            Operation::Realign(r) => r.side(),
        }
    }

    pub fn ops_total(&self) -> u8 {
        match self {
            Operation::Influence(p) => p.ops_total(),
            Operation::Realign(r) => r.ops_total(),
        }
    }

    pub fn ops_spent(&self) -> u8 {
        match self {
            Operation::Influence(p) => p.ops_spent(),
            Operation::Realign(r) => r.ops_spent(),
        }
    }

    pub fn remaining(&self) -> u8 {
        match self {
            Operation::Influence(p) => p.remaining(),
            Operation::Realign(r) => r.remaining(),
        }
    }

    /// A speculative board a view should read *instead of* the caller's
    /// real one — `Some` for a placement in progress, `None` for a
    /// realignment, whose rolls are already on the real board.
    pub fn board(&self) -> Option<&Board> {
        match self {
            Operation::Influence(p) => Some(p.board()),
            Operation::Realign(_) => None,
        }
    }

    /// Whether `id` is a legal target for the *next* action this
    /// operation would take: the next point placed, or the next roll.
    /// `board` is the caller's real board — read live by realignment,
    /// which judges legality against current state; ignored by
    /// placement, which always judges against its own frozen `base`.
    pub fn is_legal_target(&self, map: &WorldMap, board: &Board, id: CountryId) -> bool {
        match self {
            Operation::Influence(p) => p.is_legal_target(map, id),
            Operation::Realign(r) => r.is_legal_target(map, board, id),
        }
    }

    /// How much `side`'s influence in `id` has changed since this
    /// operation began, relative to `board` (the caller's real board —
    /// only read by realignment, which has no board of its own).
    /// Positive for a placement's own pending points and zero for the
    /// opponent, since placement never touches the opponent's influence;
    /// positive or negative either way for a realignment's net swing.
    pub fn delta(&self, board: &Board, id: CountryId, side: Superpower) -> i8 {
        match self {
            Operation::Influence(p) => {
                if side == p.side() {
                    p.pending(id) as i8
                } else {
                    0
                }
            }
            Operation::Realign(r) => r.delta(board, id, side),
        }
    }

    /// Whether either side's influence in `id` has changed since this
    /// operation began — the single-bit version of [`Operation::delta`]
    /// a chip glyph needs when there's no room to show a magnitude.
    pub fn touches(&self, board: &Board, id: CountryId) -> bool {
        self.delta(board, id, self.side()) != 0 || self.delta(board, id, self.side().opponent()) != 0
    }

    /// Every country this operation has touched so far, in the order it
    /// was first touched.
    pub fn touched(&self) -> Vec<CountryId> {
        match self {
            Operation::Influence(p) => p.pending_countries().into_iter().map(|(id, _)| id).collect(),
            Operation::Realign(r) => r.touched(),
        }
    }

    /// What to call this operation in a sentence — "placing" or
    /// "realigning".
    pub fn verb(&self) -> &'static str {
        match self {
            Operation::Influence(_) => "placing",
            Operation::Realign(_) => "realigning",
        }
    }

    /// Whether this operation's last action can be taken back. `false`
    /// for a realignment: a resolved die roll is permanent.
    pub fn can_undo(&self) -> bool {
        match self {
            Operation::Influence(_) => true,
            Operation::Realign(_) => false,
        }
    }
}
