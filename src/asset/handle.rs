use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

#[derive(Debug)]
pub struct Handle<T> {
    index: usize,
    _marker: PhantomData<*const T>,
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

// Manually implement Clone without requiring T: Clone
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

// Manually implement Copy without requiring T: Copy
impl<T> Copy for Handle<T> {}

// Manually implement Send and Sync since we're using raw pointers
unsafe impl<T> Send for Handle<T> {}
unsafe impl<T> Sync for Handle<T> {}

impl<T> Handle<T> {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            _marker: PhantomData,
        }
    }

    pub fn index(&self) -> usize {
        self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_is_copy() {
        let h1: Handle<String> = Handle::new(5);
        let h2 = h1;
        let h3 = h1;
        assert_eq!(h1.index(), h2.index());
        assert_eq!(h1.index(), h3.index());
    }
}
