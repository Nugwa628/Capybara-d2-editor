//! A small least-recently-used map, for the browser's thumbnail textures:
//! only what was on screen lately stays uploaded.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

pub struct Lru<K, V> {
    capacity: usize,
    tick: u64,
    items: HashMap<K, (V, u64)>,
    /// Use tick to key, oldest first.
    order: BTreeMap<u64, K>,
}

impl<K: Clone + Eq + Hash, V> Lru<K, V> {
    pub fn new(capacity: usize) -> Lru<K, V> {
        Lru {
            capacity: capacity.max(1),
            tick: 0,
            items: HashMap::new(),
            order: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    fn bump(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// The value, marked as just used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let tick = self.bump();
        let (_, used) = self.items.get_mut(key)?;
        self.order.remove(used);
        *used = tick;
        self.order.insert(tick, key.clone());
        self.items.get(key).map(|(v, _)| v)
    }

    pub fn contains(&self, key: &K) -> bool {
        self.items.contains_key(key)
    }

    /// Insert (or replace) as the newest entry; evicts the oldest entries
    /// beyond the capacity and returns them.
    pub fn insert(&mut self, key: K, value: V) -> Vec<(K, V)> {
        let tick = self.bump();
        if let Some((_, old)) = self.items.insert(key.clone(), (value, tick)) {
            self.order.remove(&old);
        }
        self.order.insert(tick, key);
        let mut evicted = Vec::new();
        while self.items.len() > self.capacity {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some((v, _)) = self.items.remove(&oldest) {
                evicted.push((oldest, v));
            }
        }
        evicted
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_used() {
        let mut lru = Lru::new(3);
        assert!(lru.insert(1, "a").is_empty());
        lru.insert(2, "b");
        lru.insert(3, "c");
        // Touch 1: now 2 is the oldest.
        assert_eq!(lru.get(&1), Some(&"a"));
        let evicted = lru.insert(4, "d");
        assert_eq!(evicted, vec![(2, "b")]);
        assert!(!lru.contains(&2));
        assert!(lru.contains(&1) && lru.contains(&3) && lru.contains(&4));
        assert_eq!(lru.len(), 3);
        // Re-inserting an entry replaces it without growing.
        assert!(lru.insert(3, "C").is_empty());
        assert_eq!(lru.get(&3), Some(&"C"));
        assert_eq!(lru.insert(5, "e"), vec![(1, "a")]);
        assert_eq!(lru.get(&9), None);
        lru.clear();
        assert_eq!(lru.len(), 0);
    }

    #[test]
    fn capacity_is_at_least_one() {
        let mut lru = Lru::new(0);
        lru.insert("x", 1);
        assert_eq!(lru.insert("y", 2), vec![("x", 1)]);
        assert_eq!(lru.len(), 1);
    }
}
