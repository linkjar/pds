//! A memory table that drops its ended entries now and then.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};

/// A `HashMap` that is swept of its ended entries once it holds `AT` or
/// more. After a sweep the next one waits until the table has doubled,
/// so a table that stays full is not swept on every call: the sweeps
/// cost a constant per insert, however many entries are live.
#[derive(Debug)]
pub struct Swept<K, V, const AT: usize> {
    map: HashMap<K, V>,
    sweep_at: usize,
}

impl<K, V, const AT: usize> Default for Swept<K, V, AT> {
    fn default() -> Self {
        Self::from(HashMap::new())
    }
}

impl<K, V, const AT: usize> From<HashMap<K, V>> for Swept<K, V, AT> {
    fn from(map: HashMap<K, V>) -> Self {
        Self { map, sweep_at: AT }
    }
}

impl<K: Eq + Hash, V, const AT: usize> Swept<K, V, AT> {
    /// Drops the entries that are not `live`, when a sweep is due.
    pub fn sweep(&mut self, mut live: impl FnMut(&V) -> bool) {
        if self.map.len() < self.sweep_at {
            return;
        }
        self.map.retain(|_, value| live(value));
        self.sweep_at = (self.map.len() * 2).max(AT);
    }
}

impl<K, V, const AT: usize> Deref for Swept<K, V, AT> {
    type Target = HashMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<K, V, const AT: usize> DerefMut for Swept<K, V, AT> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_table_is_swept_once_and_then_grows_before_the_next_sweep() {
        let mut table: Swept<u32, bool, 4> = Swept::default();
        for key in 0..4 {
            table.insert(key, key % 2 == 0);
        }
        table.sweep(|live| *live);
        assert_eq!(table.len(), 2);
        assert_eq!(table.sweep_at, 4);
        for key in 10..16 {
            table.insert(key, true);
        }
        table.sweep(|live| *live);
        assert_eq!(table.len(), 8);
        assert_eq!(table.sweep_at, 16);
        for key in 20..27 {
            table.insert(key, false);
        }
        table.sweep(|live| *live);
        assert_eq!(table.len(), 15, "a table under the mark is not swept");
        table.insert(27, false);
        table.sweep(|live| *live);
        assert_eq!(table.len(), 8);
        assert_eq!(table.sweep_at, 16);
    }
}
