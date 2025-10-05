use super::Handle;

pub struct AssetCache<T> {
    items: Vec<T>,
}

impl<T> AssetCache<T> {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn insert(&mut self, item: T) -> Handle<T> {
        let index = self.items.len();
        self.items.push(item);
        Handle::new(index)
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
}

impl<T> Default for AssetCache<T> {
    fn default() -> Self {
        Self::new()
    }
}
