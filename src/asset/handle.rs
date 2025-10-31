use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleRepr<M> {
    index: usize,
    _marker: M,
}

#[derive(Debug)]
pub struct HandleMarker<T>(PhantomData<fn() -> T>);

impl<T> HandleMarker<T> {
    const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Clone for HandleMarker<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for HandleMarker<T> {}

impl<T> PartialEq for HandleMarker<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> Eq for HandleMarker<T> {}

impl<T> Hash for HandleMarker<T> {
    fn hash<H: Hasher>(&self, _: &mut H) {}
}

pub type Handle<T> = HandleRepr<HandleMarker<T>>;

impl<M> HandleRepr<M> {
    pub fn index(&self) -> usize {
        self.index
    }
}

impl<T> HandleRepr<HandleMarker<T>> {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            _marker: HandleMarker::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn handle_is_copy() {
        let h1: Handle<String> = Handle::new(5);
        let h2 = h1;
        let h3 = h1;
        assert_eq!(h1.index(), h2.index());
        assert_eq!(h1.index(), h3.index());
    }

    #[test]
    fn handle_is_send_and_sync_for_non_send_assets() {
        assert_send_sync::<Handle<Rc<u8>>>();
    }
}
