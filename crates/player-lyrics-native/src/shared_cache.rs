//! One raster cache shared by every lyrics window (ADR 0034): a window keeps the lines it uses (`Rc`), and this
//! registry only holds weak references by raster key, so a window whose key matches (same text, font px at its DPI,
//! colour, effect, line height and translation size) reuses the surface another window drew, and an entry lives
//! exactly while some window still holds it.

use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub struct SharedCache<V> {
    map: HashMap<String, Weak<V>>,
}

impl<V> Default for SharedCache<V> {
    fn default() -> Self {
        SharedCache {
            map: HashMap::new(),
        }
    }
}

impl<V> SharedCache<V> {
    /// The live entry for `key`, if some window still holds it.
    pub fn get(&mut self, key: &str) -> Option<Rc<V>> {
        let found = self.map.get(key).and_then(Weak::upgrade);
        if found.is_none() {
            self.map.remove(key);
        }
        found
    }

    pub fn insert(&mut self, key: String, value: &Rc<V>) {
        if self.map.len() >= 64 && self.map.len().is_power_of_two() {
            self.map.retain(|_, w| w.strong_count() > 0);
        }
        self.map.insert(key, Rc::downgrade(value));
    }

    /// Forget everything (device lost: the surfaces are gone).
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// The entries some window still holds.
    pub fn live(&self) -> impl Iterator<Item = Rc<V>> + '_ {
        self.map.values().filter_map(Weak::upgrade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two taskbar windows with the same key draw a line once; a window with another key (another DPI, so another
    /// font px) draws its own; the entry goes when the last window holding it lets go.
    #[test]
    fn windows_with_the_same_key_share_and_the_last_user_frees() {
        let mut shared: SharedCache<String> = SharedCache::default();
        let mut drawn = 0;
        let mut lookup = |cache: &mut SharedCache<String>,
                          window: &mut HashMap<String, Rc<String>>,
                          key: &str| {
            if let Some(v) = window.get(key) {
                return v.clone();
            }
            let v = cache.get(key).unwrap_or_else(|| {
                drawn += 1;
                let v = Rc::new(format!("surface {key}"));
                cache.insert(key.to_owned(), &v);
                v
            });
            window.insert(key.to_owned(), v.clone());
            v
        };
        let (mut a, mut b, mut c) = (HashMap::new(), HashMap::new(), HashMap::new());
        let line_240 = "夜空|50px@240dpi|white|shadow";
        let line_192 = "夜空|40px@192dpi|white|shadow";
        let first = lookup(&mut shared, &mut a, line_240);
        let second = lookup(&mut shared, &mut b, line_240);
        assert!(Rc::ptr_eq(&first, &second));
        lookup(&mut shared, &mut c, line_192);
        assert_eq!(drawn, 2, "one per unique key");
        drop((first, second));
        a.clear();
        assert!(shared.get(line_240).is_some(), "b still uses it");
        b.clear();
        assert!(shared.get(line_240).is_none(), "the last user let go");
        assert_eq!(shared.live().count(), 1);
        shared.clear();
        assert!(shared.get(line_192).is_none());
    }
}
