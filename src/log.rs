//! The game's history — a plain, append-only record of what happened,
//! built up by [`crate::game::Game`] at the one place it already funnels
//! every mutation through. No formatting, no map, no terminal: the split
//! `ops/` already keeps between rules and display (`RollResult` lives in
//! `ops/realign.rs`, its prose rendering in `render/mod.rs`) is the same
//! split kept here — turning a [`LogEntry`] into text is `render::log`'s
//! job, not this module's.
//!
//! A turn's log reads in the order things actually happen: playing a card
//! is the first thing a turn does, so [`Event::Selected`] is pushed the
//! instant [`crate::game::Game::play_card`] succeeds — before the
//! operation it funds has even opened, let alone closed — rather than
//! being folded into a later entry as an afterthought. Every operation
//! closes with an [`Event::Closed`] entry — `confirm`'s or `cancel`'s own
//! line, naming the operation kind and its final ops balance — mirroring
//! how a realignment or coup's dice already get their own entries the
//! instant they resolve, before the operation itself closes:
//! [`crate::ops::InfluencePlacement`]'s individual points are speculative
//! until then, so they collapse into a single [`Event::Placed`] pushed
//! alongside the `Closed` entry, the placement analogue of a resolved
//! roll.

use crate::cards::CardId;
use crate::country::{CountryId, Superpower};
use crate::game::OperationKind;
use crate::ops::{CoupResult, RollResult};

/// What happened, with no opinion on how it should be displayed.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A card taken from the active side's hand via
    /// [`crate::game::Game::play_card`] — pushed immediately, on success,
    /// since playing a card is real and done the instant it happens,
    /// unlike a placement's still-speculative points. Logged even if the
    /// card is later returned ([`crate::game::Game::return_card`]) without
    /// ever funding an operation — the log says what was actually
    /// selected, not just what it led to.
    Selected { card: CardId },
    /// An influence placement's points, all at once — pushed alongside the
    /// [`Event::Closed`] entry that closes the operation, since individual
    /// points are only ever speculative until then. Omitted entirely if
    /// nothing was placed, the same way zero realignment rolls simply mean
    /// zero [`Event::Realign`] entries. Always the countries actually
    /// pending when the operation closed, whether it was confirmed or
    /// cancelled — the following `Closed` entry's action word (`confirm`
    /// vs `cancel`) is what says whether they landed on the board.
    Placed { countries: Vec<(CountryId, u8)> },
    /// One resolved realignment roll, logged the instant it resolves —
    /// before the operation that contains it has necessarily closed.
    Realign(RollResult),
    /// A coup's one attempt, logged the instant it resolves.
    Coup(CoupResult),
    /// An operation closed — `confirm`'s or `cancel`'s own entry. A
    /// realignment's or coup's rolls are already in the log as their own
    /// entries by the time this is pushed; a placement's points arrive in
    /// the immediately preceding [`Event::Placed`] entry instead, since it
    /// has no rolls of its own. Doesn't repeat which card funded it — the
    /// [`Event::Selected`] entry already named that, earlier in the same
    /// turn. `rolls` is only meaningful for `OperationKind::Realign` — a
    /// coup resolves at most one attempt and a placement none, so both
    /// leave it at 0 and the renderer ignores it for those kinds.
    Closed {
        kind: OperationKind,
        committed: bool,
        rolls: u8,
        ops_spent: u8,
        ops_total: u8,
    },
    /// The active side forfeited its turn with no operation open.
    Pass,
    /// A `set`/`add`/`remove`-style debug edit, made through
    /// [`crate::game::Game::board_mut`] — which bypasses the operation
    /// system entirely, so `Game` cannot observe it except through this
    /// being reported explicitly.
    Edit {
        country: CountryId,
        side: Superpower,
        before: u8,
        after: u8,
    },
    /// A free-text annotation — e.g. reloading the demo scenario — with no
    /// side and no board effect of its own.
    Note(String),
}

/// One thing that happened, stamped with when and by whom. `side` is
/// `None` for an event that belongs to no side (a debug edit, a note).
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub turn: u8,
    pub action_round: u8,
    pub side: Option<Superpower>,
    pub event: Event,
}

/// The game's whole history, in the order it happened.
#[derive(Debug, Clone, Default)]
pub struct GameLog {
    entries: Vec<LogEntry>,
}

impl GameLog {
    pub fn new() -> Self {
        GameLog { entries: Vec::new() }
    }

    pub fn push(&mut self, entry: LogEntry) {
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The last `n` entries, oldest first — fewer than `n` if the log
    /// doesn't have that many yet.
    pub fn tail(&self, n: usize) -> &[LogEntry] {
        let start = self.entries.len().saturating_sub(n);
        &self.entries[start..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u8) -> LogEntry {
        LogEntry { turn: 1, action_round: n, side: None, event: Event::Pass }
    }

    #[test]
    fn tail_clamps_to_the_available_entries() {
        let mut log = GameLog::new();
        for n in 1..=3 {
            log.push(entry(n));
        }
        assert_eq!(log.tail(10).len(), 3);
        assert_eq!(log.tail(2).iter().map(|e| e.action_round).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(log.tail(0).len(), 0);
    }
}
