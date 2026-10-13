//! The random source of a simulation: every draw is a function of the seed.

/// SplitMix64. It is small, it has no state besides one word, and its
/// sequence for a seed is the same on every machine.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator for `seed`.
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 bits.
    pub const fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number below `bound`. `bound` must not be zero.
    pub const fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// An index into a collection of `len` items. `len` must not be zero.
    pub fn index(&mut self, len: usize) -> usize {
        usize::try_from(self.below(len as u64)).unwrap_or(0)
    }

    /// True `percent` times in a hundred.
    pub const fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// One of `items`.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.index(items.len())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_of_a_seed_is_fixed() {
        let mut rng = Rng::new(1);
        assert_eq!(rng.next(), 0x910a_2dec_8902_5cc1);
        assert_eq!(rng.next(), 0xbeeb_8da1_658e_ec67);
        let mut again = Rng::new(1);
        assert_eq!(again.next(), 0x910a_2dec_8902_5cc1);
    }
}
