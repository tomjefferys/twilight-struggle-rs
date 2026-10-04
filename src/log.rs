//! The game's history — a plain, append-only record of what happened,
//! built up by [`crate::game::Game`] at the one place it already funnels
//! every mutation through. No formatting, no map, no terminal: the split
//! `ops/` already keeps between rules and display (`RollResult` lives in
//! `ops/realign.rs`, its prose rendering in `render/mod.rs`) is the same
//! split kept here — turning a [`LogEntry`] into text is `render::log`'s
//! job, not this module's.
//!
//! This is a record of the actual game, not of every step taken inside the
//! application — merely *selecting* a card
//! ([`crate::game::Game::play_card`]) writes nothing, since it can still
//! be taken back with no trace ([`crate::game::Game::return_card`]).
//! [`Event::Selected`] only gets pushed, by
//! [`crate::game::Game::log_card_selected`], once that selection is
//! actually irrevocable — which still lands it *before* the operation it
//! funds has necessarily closed, reading in the order things actually
//! happen rather than as an afterthought folded into a later entry. Every
//! operation closes with an [`Event::Closed`] entry — `confirm`'s or
//! `cancel`'s own line, naming the operation kind and its final ops
//! balance — mirroring how a realignment or coup's dice already get their
//! own entries the instant they resolve, before the operation itself
//! closes: [`crate::ops::InfluencePlacement`]'s individual points are
//! speculative until then, so they collapse into a single
//! [`Event::Placed`] pushed alongside the `Closed` entry, the placement
//! analogue of a resolved roll.

use crate::cards::CardId;
use crate::country::{CountryId, Superpower};
use crate::events::{EffectResult, ScoringResult, WarResult};
use crate::game::{OperationKind, Victory};
use crate::ops::{CoupResult, RollResult};
use crate::space::SpaceResult;

/// What happened, with no opinion on how it should be displayed.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A card taken from the active side's hand via
    /// [`crate::game::Game::play_card`] — but only once its selection
    /// becomes irrevocable (see [`crate::game::Game::log_card_selected`]).
    /// A card that's played and then returned
    /// ([`crate::game::Game::return_card`]) without ever reaching that
    /// point gets no entry at all: as far as the actual game is
    /// concerned, it was never really played.
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
    /// What a coup attempt set off beyond the board — the DEFCON drop of a
    /// battleground coup (or Nuclear Subs sparing it) and Yuri and
    /// Samantha's VP — pushed right after the `Coup` entry it follows.
    CoupAftermath(CoupAftermath),
    /// A war card's one roll and what it did, plus the VP track's new
    /// value — logged the instant it resolves, which is also when the
    /// event (and the turn) ends.
    War { result: WarResult, vp_after: i8 },
    /// A card spent on a space race attempt, with the VP track's new value
    /// — logged the instant it resolves, which is also when the turn ends.
    Space { result: SpaceResult, vp_after: i8 },
    /// A trapped side's action round (Bear Trap, Quagmire): the discard, the die, the outcome.
    Trap(crate::game::TrapResult),
    /// Cuban Missile Crisis ended by its threatened side removing 2 of its own influence.
    Defused { side: crate::country::Superpower, country: CountryId },
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
    /// A lasting event paid out on its own (Flower Power, We Will Bury You):
    /// the card, the VP change (US-positive) and the track's new value.
    Triggered { card: CardId, vp_delta: i8, vp_after: i8 },
    /// The active side forfeited its turn with no operation open.
    Pass,
    /// A scoring card's event resolved — [`crate::game::Game::play_event`],
    /// pushed right after the `Selected` entry for the same card (so
    /// still before `advance`, if the game didn't just end) and carrying
    /// the VP track's new value, since `result.vp_delta` alone doesn't
    /// say what it landed on.
    Scored { result: ScoringResult, vp_after: i8 },
    /// A fixed-effect card's event resolved (`events::effects`) — the
    /// same moment and position as `Scored`, with the VP track's new
    /// value for the same reason. DEFCON's own before/after is already
    /// carried by the result.
    EventResolved { result: EffectResult, vp_after: i8 },
    /// The game ended — pushed by [`crate::game::Game::play_event`]
    /// immediately after the `Scored` entry that caused it, the one
    /// entry type with no side of its own (the log's `side` field is set
    /// from the active player who happened to trigger it, but the
    /// victory itself belongs to [`crate::game::Victory::side`]).
    GameOver(Victory),
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
    /// A turn ended (rule 4.5): Military Operations settled, then the next turn dealt.
    TurnEnd(TurnEndReport),
}

/// What the end of a turn did, in order — kept whole so the log line can say all of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnEndReport {
    /// `(US, USSR)` Military Operations when the turn ended, and DEFCON they were measured against.
    pub mil_ops: (i8, i8),
    pub defcon: u8,
    /// VP the shortfalls were worth, positive favouring the US, and the track afterwards.
    pub vp_delta: i8,
    pub vp_after: i8,
    /// DEFCON `(before, after)` when it improved at the turn change.
    pub defcon_change: Option<(u8, u8)>,
    /// Cards dealt `(US, USSR)` and whether the discard pile had to be reshuffled to do it.
    pub dealt: (u8, u8),
    pub reshuffled: bool,
    /// Mid or Late War cards shuffled into the deck at this turn change.
    pub added: u8,
}

/// What a coup attempt triggered besides its own board result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoupAftermath {
    /// DEFCON `(before, after)`, when a battleground coup degraded it.
    pub defcon: Option<(u8, u8)>,
    /// Nuclear Subs spared the DEFCON drop a battleground coup would cost.
    pub defcon_spared: bool,
    /// Yuri and Samantha's VP: `(signed change, VP track after)`.
    pub vp: Option<(i8, i8)>,
    /// The coup-maker's Military Operations track `(before, after)`.
    pub mil_ops: Option<(i8, i8)>,
}

impl CoupAftermath {
    pub fn is_empty(&self) -> bool {
        *self == CoupAftermath::default()
    }
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
