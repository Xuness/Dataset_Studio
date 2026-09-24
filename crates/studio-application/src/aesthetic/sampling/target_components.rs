//! Ordered component index; preserves the original seed tie-breaks.
use super::*;
pub(super) struct TargetComponents {
    remaining: Vec<usize>,
    pub(super) sizes: BTreeMap<usize, usize>,
}
impl TargetComponents {
    pub(super) fn new(order: &[usize], parents: &mut [usize], sizes: &[usize]) -> Self {
        let mut remaining = vec![0; parents.len()];
        let mut index = BTreeMap::new();
        for &i in order {
            let r = root(parents, i);
            if remaining[r] == 0 {
                *index.entry(sizes[r]).or_insert(0) += 1;
            }
            remaining[r] += 1;
        }
        Self {
            remaining,
            sizes: index,
        }
    }
    fn subtract(&mut self, size: usize) {
        let count = self.sizes.get_mut(&size).expect("indexed component");
        *count -= 1;
        if *count == 0 {
            self.sizes.remove(&size);
        }
    }
    pub(super) fn take(&mut self, i: usize, parents: &mut [usize], sizes: &[usize]) {
        let r = root(parents, i);
        assert!(self.remaining[r] > 0);
        self.remaining[r] -= 1;
        if self.remaining[r] == 0 {
            self.subtract(sizes[r]);
        }
    }
    pub(super) fn merge(&mut self, a: usize, b: usize, parents: &mut [usize], sizes: &mut [usize]) {
        let a = root(parents, a);
        let b = root(parents, b);
        if a == b {
            return;
        }
        let n = self.remaining[a] + self.remaining[b];
        if self.remaining[a] > 0 {
            self.subtract(sizes[a]);
        }
        if self.remaining[b] > 0 {
            self.subtract(sizes[b]);
        }
        self.remaining[a] = 0;
        self.remaining[b] = 0;
        union(parents, sizes, a, b);
        let r = root(parents, a);
        self.remaining[r] = n;
        if n > 0 {
            *self.sizes.entry(sizes[r]).or_insert(0) += 1;
        }
    }
}
