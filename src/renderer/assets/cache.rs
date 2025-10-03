use super::handle::Handle;
use std::marker::PhantomData;

pub struct AssetCache<T> {
    items: Vec<T>,
    _phantom: PhantomData<T>,
}

impl<T> Default for AssetCache<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            _phantom: PhantomData,
        }
    }
}

impl<T> AssetCache<T> {
    pub fn insert(&mut self, item: T) -> Handle<T> {
        let idx = self.items.len() as u32;
        self.items.push(item);
        Handle::new(idx)
    }

    /// Get a reference to an asset by handle.
    /// Returns None if the handle is invalid.
    pub fn get(&self, h: Handle<T>) -> Option<&T> {
        self.items.get(h.idx())
    }

    /// Get a mutable reference to an asset by handle.
    /// Returns None if the handle is invalid.
    pub fn get_mut(&mut self, h: Handle<T>) -> Option<&mut T> {
        self.items.get_mut(h.idx())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_invalid_handle_returns_none() {
        let cache: AssetCache<u32> = AssetCache::default();
        let fake_handle = Handle::new(999);
        assert!(cache.get(fake_handle).is_none());
    }

    #[test]
    fn get_valid_handle_returns_some() {
        let mut cache = AssetCache::default();
        let handle = cache.insert(42u32);
        assert_eq!(cache.get(handle), Some(&42));
    }
}
