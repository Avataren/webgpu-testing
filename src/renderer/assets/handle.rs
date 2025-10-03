use std::marker::PhantomData;

// Manual impl to make it unambiguous even if derive was acting weird.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Handle<T>(u32, PhantomData<T>);

impl<T> Copy for Handle<T> {}
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Handle<T> {
    pub fn new(idx: u32) -> Self {
        Handle(idx, PhantomData)
    }
    pub fn idx(self) -> usize {
        self.0 as usize
    }
}
