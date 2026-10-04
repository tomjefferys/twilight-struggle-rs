//! A small seedable d6 source for realignment rolls.
//!
//! Not the `rand` crate — `CLAUDE.md` records crossterm as the one
//! non-serde dependency, and a hand-rolled generator is what lets every
//! realignment test drive an exact, reproducible sequence of rolls
//! instead of asserting on ranges.

/// A splitmix64-based die: cheap, seedable, and good enough for a board
/// game roll — this is not meant to be cryptographically sound.
pub struct Dice {
    state: u64,
}

impl Dice {
    /// A die whose sequence is fully determined by `seed` — the same
    /// seed always rolls the same numbers in the same order.
    pub fn from_seed(seed: u64) -> Self {
        Dice { state: seed }
    }

    /// A die seeded from the system clock, for real play.
    pub fn from_entropy() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Dice::from_seed(nanos)
    }

    /// One splitmix64 step, advancing `state` and returning the next
    /// pseudo-random `u64` — the shared generator both [`Dice::roll`] and
    /// [`Dice::index`] fold onto their own smaller range.
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// One die roll, uniform-ish over 1..=6.
    ///
    /// The result is folded onto 1..=6 by a widening multiply
    /// (`(x as u128 * 6) >> 64`) rather than `% 6`, which would bias low
    /// values. This is Lemire's trick *without* its rejection loop, so it
    /// isn't perfectly uniform either — since 2^64 isn't a multiple of 6,
    /// faces 1-4 are each ~1 part in 2^64 more likely than faces 5-6. That
    /// bias is far below anything a die roll in this game could ever
    /// expose, so the rejection loop (and the extra state-advance-on-reject
    /// it implies for reproducibility) is deliberately not worth it here.
    pub fn roll(&mut self) -> u8 {
        (((self.next_u64() as u128) * 6) >> 64) as u8 + 1
    }

    /// A uniform-ish index into `0..n`, by the same widening-multiply fold
    /// as [`Dice::roll`] (same small bias, same reasoning for why it's not
    /// worth correcting here) — what a random AI uses to pick among a
    /// slice of legal actions. Panics if `n == 0`.
    pub fn index(&mut self, n: usize) -> usize {
        assert!(n > 0, "Dice::index called with n == 0");
        (((self.next_u64() as u128) * n as u128) >> 64) as usize
    }

    /// Fisher-Yates shuffle of `items` in place, one [`Dice::index`] per swap — so a seeded
    /// `Dice` always deals the same deck.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.index(i + 1);
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_rolls_the_same_sequence() {
        let mut a = Dice::from_seed(42);
        let mut b = Dice::from_seed(42);
        let rolls_a: Vec<u8> = (0..20).map(|_| a.roll()).collect();
        let rolls_b: Vec<u8> = (0..20).map(|_| b.roll()).collect();
        assert_eq!(rolls_a, rolls_b);
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Dice::from_seed(1);
        let mut b = Dice::from_seed(2);
        let rolls_a: Vec<u8> = (0..20).map(|_| a.roll()).collect();
        let rolls_b: Vec<u8> = (0..20).map(|_| b.roll()).collect();
        assert_ne!(rolls_a, rolls_b);
    }

    #[test]
    fn every_roll_is_a_valid_die_face() {
        let mut dice = Dice::from_seed(7);
        for _ in 0..10_000 {
            let roll = dice.roll();
            assert!((1..=6).contains(&roll), "roll {roll} out of range");
        }
    }

    #[test]
    fn every_face_appears_over_many_rolls() {
        let mut dice = Dice::from_seed(123);
        let mut seen = [false; 6];
        for _ in 0..1000 {
            seen[(dice.roll() - 1) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "not every face of 1..=6 appeared in 1000 rolls: {seen:?}");
    }

    #[test]
    fn index_stays_in_range() {
        let mut dice = Dice::from_seed(17);
        for n in 1..=20 {
            for _ in 0..1000 {
                assert!(dice.index(n) < n);
            }
        }
    }

    #[test]
    fn index_is_reproducible_from_the_same_seed() {
        let mut a = Dice::from_seed(42);
        let mut b = Dice::from_seed(42);
        let a: Vec<usize> = (0..20).map(|_| a.index(7)).collect();
        let b: Vec<usize> = (0..20).map(|_| b.index(7)).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn the_distribution_is_roughly_even() {
        let mut dice = Dice::from_seed(999);
        let mut counts = [0u32; 6];
        let n = 600_000;
        for _ in 0..n {
            counts[(dice.roll() - 1) as usize] += 1;
        }
        let expected = n as f64 / 6.0;
        for (face, &count) in counts.iter().enumerate() {
            let deviation = (count as f64 - expected).abs() / expected;
            assert!(deviation < 0.01, "face {} deviated by {:.2}% (count {count}, expected {expected})", face + 1, deviation * 100.0);
        }
    }

    #[test]
    fn shuffle_keeps_every_item_and_is_reproducible() {
        let mut a: Vec<u8> = (0..30).collect();
        let mut b = a.clone();
        Dice::from_seed(9).shuffle(&mut a);
        Dice::from_seed(9).shuffle(&mut b);
        assert_eq!(a, b);
        assert_ne!(a, (0..30).collect::<Vec<u8>>(), "30 items staying in order would be astronomically unlikely");
        a.sort();
        assert_eq!(a, (0..30).collect::<Vec<u8>>());
    }
}
