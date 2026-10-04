//! Tiny deterministic PRNG (xorshift64*). No external crate and no wall clock.

#[derive(Clone, Debug)]
pub struct Rng(u64);

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

impl Rng {
    /// Derive a generator from several identifying numbers (seed, conversation, index, ...).
    pub fn from_parts(parts: &[u64]) -> Self {
        let mut s = 0x1234_5678_9ABC_DEF1u64;
        for p in parts {
            s = splitmix(s ^ splitmix(*p));
        }
        Rng(if s == 0 { 1 } else { s })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish value in `0..n` (`n` must be non-zero).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() >> 11) as usize % n.max(1)
    }

    /// Inclusive range.
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_parts_same_sequence() {
        let mut a = Rng::from_parts(&[1, 2, 3]);
        let mut b = Rng::from_parts(&[1, 2, 3]);
        let mut c = Rng::from_parts(&[1, 2, 4]);
        let va: Vec<_> = (0..8).map(|_| a.next_u64()).collect();
        let vb: Vec<_> = (0..8).map(|_| b.next_u64()).collect();
        let vc: Vec<_> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(va, vb);
        assert_ne!(va, vc);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::from_parts(&[9]);
        assert!((0..1000).all(|_| r.below(7) < 7));
        assert!((0..1000).all(|_| (3..=5).contains(&r.range(3, 5))));
    }
}
