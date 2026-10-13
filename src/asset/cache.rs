use super::Handle;

pub struct AssetCache<T> {
    items: Vec<T>,
    /// Released slots, reused by the next inserts. A released item stays in place
    /// until its slot is reused, so a stale handle still reads valid data.
    free: Vec<usize>,
    released: Vec<bool>,
}

impl<T> Default for AssetCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> AssetCache<T> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            free: Vec::new(),
            released: Vec::new(),
        }
    }

    pub fn insert(&mut self, item: T) -> Handle<T> {
        if let Some(index) = self.free.pop() {
            self.items[index] = item;
            self.released[index] = false;
            return Handle::new(index);
        }
        let index = self.items.len();
        self.items.push(item);
        self.released.push(false);
        Handle::new(index)
    }

    /// Give the slot back for reuse. The caller must know no handle to it is still in
    /// use: a later insert will put another item there.
    pub fn release(&mut self, handle: Handle<T>) -> bool {
        let index = handle.index();
        match self.released.get_mut(index) {
            Some(released) if !*released => {
                *released = true;
                self.free.push(index);
                true
            }
            _ => false,
        }
    }

    pub fn is_released(&self, handle: Handle<T>) -> bool {
        self.released.get(handle.index()).copied().unwrap_or(false)
    }

    /// Slots in use (not released).
    pub fn live_count(&self) -> usize {
        self.items.len() - self.free.len()
    }

    /// Live items with their handles.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.items
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.released[*index])
            .map(|(index, item)| (Handle::new(index), item))
    }

    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        self.items.get(handle.index())
    }

    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.items.get_mut(handle.index())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn into_inner(self) -> Vec<T> {
        self.items
    }
}

impl<T: Clone> Clone for AssetCache<T> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            free: self.free.clone(),
            released: self.released.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_slots_are_reused() {
        let mut cache = AssetCache::new();
        let a = cache.insert("a");
        let b = cache.insert("b");
        assert!(cache.release(a));
        assert!(!cache.release(a), "a slot is released once");
        assert_eq!(cache.live_count(), 1);
        assert_eq!(
            cache.iter().map(|(_, item)| *item).collect::<Vec<_>>(),
            ["b"]
        );
        let c = cache.insert("c");
        assert_eq!(c.index(), a.index());
        assert_eq!(cache.get(c), Some(&"c"));
        assert_eq!(cache.get(b), Some(&"b"));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.live_count(), 2);
    }
}
