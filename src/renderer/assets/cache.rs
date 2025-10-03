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
    pub fn get(&self, h: Handle<T>) -> &T {
        &self.items[h.idx()]
    }
    pub fn get_mut(&mut self, h: Handle<T>) -> &mut T {
        &mut self.items[h.idx()]
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
}
