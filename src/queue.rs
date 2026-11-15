//! The priority queue of the path searches.
//!
//! A search takes states out in order of estimated total cost, and that estimate never falls as the search
//! goes on. Keys are whole numbers (the cost in 64ths of a grid step).
//!
//! Two earlier queues, measured on the bus benchmark: the standard binary heap took 57 % of the search time,
//! most of it fetching scattered memory while sifting; a radix heap (buckets by the highest bit in which a key
//! differs from the last one taken out) halved the search time, but moved each entry about nine times on its
//! way out.

/// A bucket queue: one bucket per key over a window of `NEAR` keys ahead of the last key
/// taken out, and one unsorted heap of everything further ahead, brought in when the window has run empty.
/// Putting in and taking out touch one bucket; nothing is moved while it waits (in the radix heap each entry
/// was moved about nine times on the bus benchmark). Of equal keys the last put in comes out first, which in a
/// path search is the state furthest along.
pub struct BucketQueue {
    near: Vec<Vec<u32>>,
    filled: Vec<u64>,
    base: u64,
    cursor: usize,
    far: Vec<(u64, u32)>,
    len: usize,
}

const NEAR: usize = 1 << 14;

impl BucketQueue {
    pub fn new() -> BucketQueue {
        BucketQueue { near: (0..NEAR).map(|_| Vec::new()).collect(), filled: vec![0; NEAR / 64], base: 0, cursor: 0, far: Vec::new(), len: 0 }
    }

    pub fn clear(&mut self) {
        for (word, bits) in self.filled.iter_mut().enumerate() {
            while *bits != 0 {
                let b = bits.trailing_zeros() as usize;
                *bits &= *bits - 1;
                self.near[word * 64 + b].clear();
            }
        }
        self.far.clear();
        self.base = 0;
        self.cursor = 0;
        self.len = 0;
    }

    /// Put in a value. A key below the last one taken out is treated as equal to it.
    #[inline]
    pub fn push(&mut self, key: u64, value: u32) {
        let at = key.saturating_sub(self.base).max(self.cursor as u64);
        if at < NEAR as u64 {
            let at = at as usize;
            self.near[at].push(value);
            self.filled[at / 64] |= 1 << (at % 64);
        } else {
            self.far.push((key, value));
        }
        self.len += 1;
    }

    /// Take out a value with the smallest key.
    #[inline]
    pub fn pop(&mut self) -> Option<(u64, u32)> {
        if self.len == 0 {
            return None;
        }
        loop {
            let mut word = self.cursor / 64;
            let mut bits = self.filled[word] & (!0u64 << (self.cursor % 64));
            while bits == 0 && word + 1 < NEAR / 64 {
                word += 1;
                bits = self.filled[word];
            }
            if bits != 0 {
                let at = word * 64 + bits.trailing_zeros() as usize;
                self.cursor = at;
                let value = self.near[at].pop().unwrap();
                if self.near[at].is_empty() {
                    self.filled[word] &= !(1 << (at % 64));
                }
                self.len -= 1;
                return Some((self.base + at as u64, value));
            }
            // the window is empty: move it to the smallest key waiting further ahead
            self.base = self.far.iter().map(|e| e.0).min().unwrap();
            self.cursor = 0;
            let base = self.base;
            let (near, filled) = (&mut self.near, &mut self.filled);
            self.far.retain(|&(key, value)| {
                let at = (key - base) as usize;
                if at < NEAR {
                    near[at].push(value);
                    filled[at / 64] |= 1 << (at % 64);
                    false
                } else {
                    true
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_come_out_in_order() {
        let mut queue = BucketQueue::new();
        let mut seed = 99u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed >> 40
        };
        let (mut floor, mut out, mut put) = (0u64, 0usize, 0usize);
        for round in 0..3000 {
            for _ in 0..5 {
                // mostly near, some far beyond the window, some below the last key taken out
                let key = match next() % 10 {
                    0 => floor + 20000 + next() % 100000,
                    1 => floor.saturating_sub(next() % 50),
                    _ => floor + next() % 3000,
                };
                queue.push(key, round);
                put += 1;
            }
            for _ in 0..3 {
                let (key, _) = queue.pop().unwrap();
                assert!(key >= floor);
                floor = key;
                out += 1;
            }
            if round == 1500 {
                queue.clear();
                floor = 0;
                put = out;
            }
        }
        while let Some((key, _)) = queue.pop() {
            assert!(key >= floor);
            floor = key;
            out += 1;
        }
        assert_eq!(out, put);
    }
}
