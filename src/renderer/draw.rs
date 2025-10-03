use super::assets::{Handle, Mesh};
use std::ops::Range;

pub struct DrawItem {
    pub mesh: Handle<Mesh>,
    pub object_range: Range<u32>, // instances to draw: e.g. 0..N
}
