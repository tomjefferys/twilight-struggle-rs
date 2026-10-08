//! [`SearchAi`]: determinized Monte Carlo search over whole action rounds.
//!
//! The decisions that matter most — which card to play and how (event, influence, realignment,
//! coup, space), and which card to headline — are *searched*: every candidate is tried in many
//! sampled worlds ([`Game::determinize`] reshuffles what the AI cannot see: the opponent's hand
//! and the deck), played forward for a few action rounds by both sides with the cheap
//! [`HeuristicAi`] policy, and scored with [`evaluate`]. Successive halving spends the budget on
//! the candidates still in contention, and every candidate sees the *same* sampled worlds, so
//! the comparison between them is far less noisy than the dice alone. The small decisions inside
//! an operation (which country gets the next point) are left to the heuristic, so stepping
//! through the AI's turn in the interactive map stays instant; only the card choice thinks.
//!
//! Tuning notes (24 games as the US against `HeuristicAi`, which beats an equal opponent there
//! ~39% of the time): a 6-round horizon won 14, 12 rounds won 20, playing to the turn's end 19;
//! 900 sims at 6 rounds won 20, 600 sims at 12 rounds 18. The horizon matters; past a few hundred
//! simulations more of them do not, so the limit is the playout policy and the evaluation.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::action::Action;
use crate::cards::{CardCatalog, CardId};
use crate::country::Superpower;
use crate::dice::Dice;
use crate::game::{Game, Phase};
use crate::map::WorldMap;

use super::eval::evaluate_deep;
use super::heuristic::HeuristicAi;
use super::random::sensible;
use super::Ai;

/// The default number of action rounds (either side's) a simulation plays past the candidate
/// before scoring.
const PLIES: u8 = 12;
/// A defensive cap on steps in one simulation.
const MAX_SIM_STEPS: usize = 800;
/// At most this many candidates enter the search; the rest are pruned by a one-world pre-check.
const MAX_CANDIDATES: usize = 14;

/// What one search may spend.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    /// Total simulations across all candidates (the search is deterministic given this alone).
    pub sims: usize,
    /// A wall-clock ceiling; reaching it ends the search with whatever has been learned.
    pub time: Option<Duration>,
    /// Worker threads.
    pub threads: usize,
}

impl Budget {
    /// Interactive play: a couple of seconds on every core.
    pub fn interactive() -> Budget {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).min(8);
        Budget { sims: 400, time: Some(Duration::from_millis(2500)), threads }
    }

    /// A fixed, reproducible budget (no clock) — for tests and tuning matches.
    pub fn fixed(sims: usize) -> Budget {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).min(8);
        Budget { sims, time: None, threads }
    }
}

pub struct SearchAi {
    rng: Dice,
    budget: Budget,
    plies: u8,
    fallback: HeuristicAi,
    /// The way to play the card just chosen, applied on the next call without searching again.
    plan: Option<(CardId, Action)>,
    /// How long the last search took (for tests and tuning).
    pub last_think: Duration,
}

impl SearchAi {
    pub fn from_seed(seed: u64) -> Self {
        SearchAi { rng: Dice::from_seed(seed), budget: Budget::interactive(), plies: PLIES, fallback: HeuristicAi::from_seed(seed ^ 0xA11), plan: None, last_think: Duration::ZERO }
    }

    pub fn from_entropy() -> Self {
        let seed = Dice::from_entropy().index(usize::MAX) as u64;
        SearchAi::from_seed(seed)
    }

    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// How many action rounds each simulation plays before it is scored.
    pub fn with_plies(mut self, plies: u8) -> Self {
        self.plies = plies;
        self
    }

    /// Whether the next [`Ai::choose`] will search (and so may take a moment).
    pub fn will_search(&self, game: &Game, legal: &[Action]) -> bool {
        self.planned(game, legal).is_none() && legal.iter().filter(|a| matches!(a, Action::PlayCard(_) | Action::Headline(_))).count() > 1
    }

    fn planned(&self, game: &Game, legal: &[Action]) -> Option<Action> {
        let (card, action) = self.plan?;
        (game.card_in_play() == Some(card) && game.operation().is_none() && legal.contains(&action)).then_some(action)
    }
}

/// One thing to try at the root: the actions to apply in order (`PlayCard` then its usage, or a
/// lone `Headline`), and the first is what [`Ai::choose`] returns.
#[derive(Clone)]
struct Candidate {
    steps: Vec<Action>,
}

fn candidates(game: &Game, map: &WorldMap, cards: &CardCatalog, options: &[Action]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for &first in options {
        match first {
            Action::PlayCard(_) => {
                let mut copy = game.lookahead();
                if copy.apply(first, map, cards, &mut Dice::from_seed(0)).is_err() {
                    continue;
                }
                let usages: Vec<Action> = copy.legal_actions(map, cards).into_iter().filter(|a| matches!(a, Action::Begin(_) | Action::Event | Action::Space | Action::Pass)).collect();
                if usages.is_empty() {
                    out.push(Candidate { steps: vec![first] });
                }
                for u in usages {
                    out.push(Candidate { steps: vec![first, u] });
                }
            }
            Action::Headline(_) => out.push(Candidate { steps: vec![first] }),
            _ => {}
        }
    }
    out
}

/// A key that changes whenever a new action round (of either side) starts.
fn round_key(game: &Game) -> (u8, u8, Superpower) {
    let s = game.status();
    (s.turn, s.action_round, s.active)
}

/// Plays `steps` on a world sampled from `me`'s point of view with `seed`, lets both sides carry
/// on with the heuristic policy for [`PLIES`] action rounds (or to the turn's end), and scores
/// the result for `me`.
fn simulate(game: &Game, steps: &[Action], plies_max: u8, me: Superpower, map: &WorldMap, cards: &CardCatalog, seed: u64) -> f32 {
    let mut dice = Dice::from_seed(seed);
    let mut world = game.determinize(me, &mut dice);
    for &a in steps {
        if world.apply(a, map, cards, &mut dice).is_err() {
            return f32::NEG_INFINITY;
        }
    }
    let mut policy = HeuristicAi::from_seed(seed ^ 0x51);
    let start_turn = world.status().turn;
    let mut plies = 0;
    let mut key = round_key(&world);
    for _ in 0..MAX_SIM_STEPS {
        if world.winner().is_some() || plies >= plies_max || world.status().turn != start_turn {
            break;
        }
        let legal = world.legal_actions(map, cards);
        if legal.is_empty() {
            break;
        }
        let a = policy.choose(&world, map, cards, &legal);
        if world.apply(a, map, cards, &mut dice).is_err() {
            break;
        }
        let now = round_key(&world);
        if now != key && world.phase() == Phase::ActionRounds {
            plies += 1;
        }
        key = now;
    }
    evaluate_deep(&world, map, cards, me)
}

#[derive(Clone, Copy, Default)]
struct Stat {
    sum: f64,
    n: u32,
}

impl Stat {
    fn mean(&self) -> f64 {
        if self.n == 0 { f64::NEG_INFINITY } else { self.sum / self.n as f64 }
    }
}

/// Runs `jobs` (candidate index, world index) across the budget's threads, adding each score to
/// `stats`. Stops early at `deadline`.
#[allow(clippy::too_many_arguments)]
fn run_jobs(game: &Game, cands: &[Candidate], plies: u8, jobs: &[(usize, u32)], stats: &mut [Stat], me: Superpower, map: &WorldMap, cards: &CardCatalog, base: u64, threads: usize, deadline: Option<Instant>) {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(jobs.len()));
    let work = || {
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= jobs.len() || deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
            let (c, w) = jobs[i];
            let seed = base.wrapping_add((w as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let score = simulate(game, &cands[c].steps, plies, me, map, cards, seed);
            results.lock().unwrap().push((c, score));
        }
    };
    if threads <= 1 {
        work();
    } else {
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(work);
            }
        });
    }
    for (c, score) in results.into_inner().unwrap() {
        // A refused candidate scores -inf; keep it from poisoning the mean but never best.
        let score = if score.is_finite() { score as f64 } else { -1.0e6 };
        stats[c].sum += score;
        stats[c].n += 1;
    }
}

impl SearchAi {
    fn search(&mut self, game: &Game, map: &WorldMap, cards: &CardCatalog, options: &[Action]) -> Action {
        let me = game.decider();
        let cands = candidates(game, map, cards, options);
        if cands.len() <= 1 {
            return cands.first().map_or(options[0], |c| c.steps[0]);
        }
        let started = Instant::now();
        let deadline = self.budget.time.map(|t| started + t);
        let base = self.rng.index(usize::MAX) as u64;
        let mut stats = vec![Stat::default(); cands.len()];
        let mut alive: Vec<usize> = (0..cands.len()).collect();

        // Successive halving: each stage gets an equal share of the budget, split over the
        // survivors, each of whom sees worlds `0..n` (the same worlds for everyone).
        let stages = (usize::BITS - (alive.len().max(2) - 1).leading_zeros()) as usize;
        let per_stage = (self.budget.sims / stages).max(alive.len());
        // A cheap first cut when there are many candidates: one world each.
        if alive.len() > MAX_CANDIDATES {
            let jobs: Vec<(usize, u32)> = alive.iter().map(|&c| (c, 0)).collect();
            run_jobs(game, &cands, self.plies, &jobs, &mut stats, me, map, cards, base, self.budget.threads, deadline);
            alive.sort_by(|&a, &b| stats[b].mean().total_cmp(&stats[a].mean()));
            alive.truncate(MAX_CANDIDATES);
        }
        loop {
            let each = (per_stage / alive.len()).max(1) as u32;
            // World-major order: if the clock cuts a stage short, every survivor has had
            // (nearly) the same worlds, not some of them all and others none.
            let mut jobs = Vec::new();
            let first = alive.iter().map(|&c| stats[c].n).min().unwrap_or(0);
            for w in first..first + each {
                for &c in &alive {
                    if w >= stats[c].n {
                        jobs.push((c, w));
                    }
                }
            }
            run_jobs(game, &cands, self.plies, &jobs, &mut stats, me, map, cards, base, self.budget.threads, deadline);
            if alive.len() == 1 || deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
            alive.sort_by(|&a, &b| stats[b].mean().total_cmp(&stats[a].mean()));
            let keep = alive.len().div_ceil(2);
            alive.truncate(keep);
            if alive.len() == 1 {
                // The winner is decided; further samples would only confirm it.
                break;
            }
        }
        self.last_think = started.elapsed();
        let best = alive.into_iter().max_by(|&a, &b| stats[a].mean().total_cmp(&stats[b].mean())).unwrap_or(0);
        let chosen = &cands[best];
        if let (Action::PlayCard(card), Some(&usage)) = (chosen.steps[0], chosen.steps.get(1)) {
            self.plan = Some((card, usage));
        }
        chosen.steps[0]
    }
}

impl Ai for SearchAi {
    fn choose(&mut self, game: &Game, map: &WorldMap, cards: &CardCatalog, legal: &[Action]) -> Action {
        if let Some(action) = self.planned(game, legal) {
            self.plan = None;
            return action;
        }
        self.plan = None;
        let options = sensible(game, map, cards, legal);
        if options.len() == 1 {
            return options[0];
        }
        if options.iter().all(|a| matches!(a, Action::PlayCard(_) | Action::Headline(_))) {
            return self.search(game, map, cards, &options);
        }
        self.fallback.choose(game, map, cards, legal)
    }

    fn is_slow(&self, game: &Game, legal: &[Action]) -> bool {
        self.will_search(game, legal)
    }
}
