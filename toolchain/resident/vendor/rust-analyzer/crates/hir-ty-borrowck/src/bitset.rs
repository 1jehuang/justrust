//! A fixed-size set of small integers, for the dataflow analyses.

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(super) struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    pub(super) fn new(size: usize) -> Self {
        BitSet { words: vec![0; size.div_ceil(64)] }
    }

    pub(super) fn contains(&self, bit: usize) -> bool {
        self.words.get(bit / 64).is_some_and(|word| word & (1 << (bit % 64)) != 0)
    }

    pub(super) fn insert(&mut self, bit: usize) -> bool {
        let word = &mut self.words[bit / 64];
        let before = *word;
        *word |= 1 << (bit % 64);
        before != *word
    }

    pub(super) fn remove(&mut self, bit: usize) {
        self.words[bit / 64] &= !(1 << (bit % 64));
    }

    /// Adds the bits of `other`, and says whether that added any.
    pub(super) fn union(&mut self, other: &BitSet) -> bool {
        let mut changed = false;
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            let before = *word;
            *word |= other;
            changed |= before != *word;
        }
        changed
    }

    /// Keeps only the bits `other` has too.
    pub(super) fn intersect(&mut self, other: &BitSet) {
        for (index, word) in self.words.iter_mut().enumerate() {
            *word &= other.words.get(index).copied().unwrap_or(0);
        }
    }

    pub(super) fn clear(&mut self) {
        self.words.fill(0);
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(index, &word)| {
            let mut word = word;
            std::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let bit = word.trailing_zeros() as usize;
                word &= word - 1;
                Some(index * 64 + bit)
            })
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.words.iter().all(|it| *it == 0)
    }
}
