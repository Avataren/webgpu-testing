//! The compact heightfield: the open space above each walkable span (where an agent
//! could stand), linked to its neighbours an agent can step to. Erosion by the agent's
//! radius, area marking and obstacle carving work on it.

use glam::{Vec2, Vec3};

use super::heightfield::Heightfield;
use super::{dir_offset_x, dir_offset_z, NOT_CONNECTED, NULL_AREA, SPAN_MAX_HEIGHT};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CompactCell {
    /// Index of the column's first span in [`CompactHeightfield::spans`].
    pub index: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CompactSpan {
    /// Floor height (cells).
    pub y: u16,
    /// Region id (0: none); [`super::BORDER_REG`] marks the tile's border regions.
    pub reg: u16,
    /// The neighbour span index (within its column) per direction, 6 bits each.
    pub con: u32,
    /// Free height above the floor (cells, up to 255).
    pub h: u8,
    /// The floor's exact height (m).
    pub top: f32,
}

impl CompactSpan {
    pub fn con(&self, dir: usize) -> u8 {
        ((self.con >> (dir * 6)) & 0x3f) as u8
    }

    pub fn set_con(&mut self, dir: usize, index: u8) {
        let shift = dir * 6;
        self.con = (self.con & !(0x3f << shift)) | (u32::from(index & 0x3f) << shift);
    }

    pub fn connected(&self, dir: usize) -> bool {
        self.con(dir) != NOT_CONNECTED
    }
}

#[derive(Clone)]
pub(crate) struct CompactHeightfield {
    pub width: i32,
    pub height: i32,
    pub walkable_height: i32,
    pub walkable_climb: i32,
    pub border_size: i32,
    pub max_distance: u16,
    pub max_regions: u16,
    pub bmin: Vec3,
    pub bmax: Vec3,
    pub cs: f32,
    pub ch: f32,
    pub cells: Vec<CompactCell>,
    pub spans: Vec<CompactSpan>,
    pub dist: Vec<u16>,
    pub areas: Vec<u8>,
}

impl CompactHeightfield {
    /// The open spans of `hf`'s walkable spans, linked where an agent `walkable_height`
    /// tall can step up or down `walkable_climb` cells between them.
    pub fn build(hf: &Heightfield, walkable_height: i32, walkable_climb: i32) -> Self {
        let w = hf.width;
        let h = hf.height;
        let mut cells = vec![CompactCell::default(); (w * h).max(0) as usize];
        let mut spans = Vec::new();
        let mut areas = Vec::new();
        let mut bmax = hf.bmax;
        bmax.y += walkable_height as f32 * hf.ch;
        for z in 0..h {
            for x in 0..w {
                let column = hf.column(x, z);
                let cell = &mut cells[(x + z * w) as usize];
                cell.index = spans.len() as u32;
                for (index, span) in column.iter().enumerate() {
                    if span.area == NULL_AREA {
                        continue;
                    }
                    let bottom = i32::from(span.smax);
                    let top = column
                        .get(index + 1)
                        .map_or(SPAN_MAX_HEIGHT, |next| i32::from(next.smin));
                    spans.push(CompactSpan {
                        y: bottom.clamp(0, 0xffff) as u16,
                        reg: 0,
                        con: 0,
                        h: (top - bottom).clamp(0, 0xff) as u8,
                        top: span.top,
                    });
                    areas.push(span.area);
                    cell.count += 1;
                }
            }
        }
        let mut chf = Self {
            width: w,
            height: h,
            walkable_height,
            walkable_climb,
            border_size: 0,
            max_distance: 0,
            max_regions: 0,
            bmin: hf.bmin,
            bmax,
            cs: hf.cs,
            ch: hf.ch,
            dist: vec![0; spans.len()],
            cells,
            spans,
            areas,
        };
        chf.link_neighbors();
        chf
    }

    fn link_neighbors(&mut self) {
        let w = self.width;
        let h = self.height;
        for z in 0..h {
            for x in 0..w {
                let cell = self.cells[(x + z * w) as usize];
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    for dir in 0..4 {
                        self.spans[i].set_con(dir, NOT_CONNECTED);
                        let nx = x + dir_offset_x(dir);
                        let nz = z + dir_offset_z(dir);
                        if nx < 0 || nz < 0 || nx >= w || nz >= h {
                            continue;
                        }
                        let span = self.spans[i];
                        let neighbor_cell = self.cells[(nx + nz * w) as usize];
                        for k in 0..neighbor_cell.count {
                            let other = self.spans[(neighbor_cell.index + k) as usize];
                            let bottom = i32::from(span.y).max(i32::from(other.y));
                            let top = (i32::from(span.y) + i32::from(span.h))
                                .min(i32::from(other.y) + i32::from(other.h));
                            if top - bottom >= self.walkable_height
                                && (i32::from(other.y) - i32::from(span.y)).abs()
                                    <= self.walkable_climb
                            {
                                if k < u32::from(NOT_CONNECTED) {
                                    self.spans[i].set_con(dir, k as u8);
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn span_count(&self) -> usize {
        self.spans.len()
    }

    pub fn cell(&self, x: i32, z: i32) -> CompactCell {
        self.cells[(x + z * self.width) as usize]
    }

    /// The span index of `span`'s neighbour in direction `dir` (from cell x, z).
    pub fn neighbor_index(&self, x: i32, z: i32, span: &CompactSpan, dir: usize) -> usize {
        let nx = x + dir_offset_x(dir);
        let nz = z + dir_offset_z(dir);
        (self.cell(nx, nz).index + u32::from(span.con(dir))) as usize
    }

    /// Shrink the walkable area by `radius` cells from its edges, so an agent's centre
    /// kept on what is left leaves its body clear of walls and drops.
    pub fn erode_walkable_area(&mut self, radius: i32) {
        let w = self.width;
        let h = self.height;
        let count = self.spans.len();
        let mut dist = vec![0xffu8; count];

        // Edge spans: unwalkable, or missing a walkable neighbour.
        for z in 0..h {
            for x in 0..w {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    if self.areas[i] == NULL_AREA {
                        dist[i] = 0;
                        continue;
                    }
                    let span = self.spans[i];
                    let neighbours = (0..4)
                        .filter(|&dir| {
                            span.connected(dir)
                                && self.areas[self.neighbor_index(x, z, &span, dir)] != NULL_AREA
                        })
                        .count();
                    if neighbours != 4 {
                        dist[i] = 0;
                    }
                }
            }
        }

        let relax = |dist: &mut [u8], i: usize, j: usize, cost: u8| {
            let nd = dist[j].saturating_add(cost);
            if nd < dist[i] {
                dist[i] = nd;
            }
        };
        // Pass 1: from the -x and -z sides.
        for z in 0..h {
            for x in 0..w {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    let span = self.spans[i];
                    if span.connected(0) {
                        let ai = self.neighbor_index(x, z, &span, 0);
                        relax(&mut dist, i, ai, 2);
                        let a = self.spans[ai];
                        if a.connected(3) {
                            let aai = self.neighbor_index(x + dir_offset_x(0), z, &a, 3);
                            relax(&mut dist, i, aai, 3);
                        }
                    }
                    if span.connected(3) {
                        let ai = self.neighbor_index(x, z, &span, 3);
                        relax(&mut dist, i, ai, 2);
                        let a = self.spans[ai];
                        if a.connected(2) {
                            let aai = self.neighbor_index(x, z + dir_offset_z(3), &a, 2);
                            relax(&mut dist, i, aai, 3);
                        }
                    }
                }
            }
        }
        // Pass 2: from the +x and +z sides.
        for z in (0..h).rev() {
            for x in (0..w).rev() {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    let span = self.spans[i];
                    if span.connected(2) {
                        let ai = self.neighbor_index(x, z, &span, 2);
                        relax(&mut dist, i, ai, 2);
                        let a = self.spans[ai];
                        if a.connected(1) {
                            let aai = self.neighbor_index(x + dir_offset_x(2), z, &a, 1);
                            relax(&mut dist, i, aai, 3);
                        }
                    }
                    if span.connected(1) {
                        let ai = self.neighbor_index(x, z, &span, 1);
                        relax(&mut dist, i, ai, 2);
                        let a = self.spans[ai];
                        if a.connected(0) {
                            let aai = self.neighbor_index(x, z + dir_offset_z(1), &a, 0);
                            relax(&mut dist, i, aai, 3);
                        }
                    }
                }
            }
        }

        let threshold = (radius * 2).clamp(0, 255) as u8;
        for (area, d) in self.areas.iter_mut().zip(&dist) {
            if *d < threshold {
                *area = NULL_AREA;
            }
        }
    }

    /// The cell range (inclusive) and floor range a world box covers, or `None` outside.
    fn cell_range(&self, bmin: Vec3, bmax: Vec3) -> Option<(i32, i32, i32, i32, i32, i32)> {
        let min_x = ((bmin.x - self.bmin.x) / self.cs).floor() as i32;
        let min_z = ((bmin.z - self.bmin.z) / self.cs).floor() as i32;
        let max_x = ((bmax.x - self.bmin.x) / self.cs).floor() as i32;
        let max_z = ((bmax.z - self.bmin.z) / self.cs).floor() as i32;
        let min_y = ((bmin.y - self.bmin.y) / self.ch).floor() as i32;
        let max_y = ((bmax.y - self.bmin.y) / self.ch).ceil() as i32;
        if max_x < 0 || min_x >= self.width || max_z < 0 || min_z >= self.height {
            return None;
        }
        Some((
            min_x.max(0),
            max_x.min(self.width - 1),
            min_z.max(0),
            max_z.min(self.height - 1),
            min_y,
            max_y,
        ))
    }

    /// Set the area of walkable spans whose floor lies inside the convex prism over
    /// `polygon` (world x/z, any winding) between `min_y` and `max_y`. Area
    /// [`NULL_AREA`] carves a hole.
    pub fn mark_convex_area(&mut self, polygon: &[Vec2], min_y: f32, max_y: f32, area: u8) {
        if polygon.len() < 3 {
            return;
        }
        let (mut lo, mut hi) = (polygon[0], polygon[0]);
        for p in polygon {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let Some((x0, x1, z0, z1, y0, y1)) =
            self.cell_range(Vec3::new(lo.x, min_y, lo.y), Vec3::new(hi.x, max_y, hi.y))
        else {
            return;
        };
        for z in z0..=z1 {
            for x in x0..=x1 {
                let center = Vec2::new(
                    self.bmin.x + (x as f32 + 0.5) * self.cs,
                    self.bmin.z + (z as f32 + 0.5) * self.cs,
                );
                if !point_in_convex(center, polygon) {
                    continue;
                }
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    if self.areas[i] == NULL_AREA {
                        continue;
                    }
                    let y = i32::from(self.spans[i].y);
                    if y >= y0 && y <= y1 {
                        self.areas[i] = area;
                    }
                }
            }
        }
    }

    /// Set the area of walkable spans whose floor lies in an upright cylinder.
    pub fn mark_cylinder_area(&mut self, center: Vec3, radius: f32, height: f32, area: u8) {
        let bmin = Vec3::new(center.x - radius, center.y, center.z - radius);
        let bmax = Vec3::new(center.x + radius, center.y + height, center.z + radius);
        let Some((x0, x1, z0, z1, y0, y1)) = self.cell_range(bmin, bmax) else {
            return;
        };
        let radius_sq = radius * radius;
        for z in z0..=z1 {
            for x in x0..=x1 {
                let dx = self.bmin.x + (x as f32 + 0.5) * self.cs - center.x;
                let dz = self.bmin.z + (z as f32 + 0.5) * self.cs - center.z;
                if dx * dx + dz * dz > radius_sq {
                    continue;
                }
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    if self.areas[i] == NULL_AREA {
                        continue;
                    }
                    let y = i32::from(self.spans[i].y);
                    if y >= y0 && y <= y1 {
                        self.areas[i] = area;
                    }
                }
            }
        }
    }
}

/// Whether `p` lies inside the convex polygon (either winding).
pub(crate) fn point_in_convex(p: Vec2, polygon: &[Vec2]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let cross = (b - a).perp_dot(p - a);
        if cross.abs() <= f32::EPSILON {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::WALKABLE_AREA;
    use super::*;

    fn floor(size: i32) -> CompactHeightfield {
        let mut hf = Heightfield::new(
            size,
            size,
            Vec3::ZERO,
            Vec3::new(size as f32, 4.0, size as f32),
            1.0,
            0.1,
        );
        for z in 0..size {
            for x in 0..size {
                hf.add_span(x, z, 0, 2, WALKABLE_AREA, 0.2, 1);
            }
        }
        CompactHeightfield::build(&hf, 18, 4)
    }

    #[test]
    fn neighbours_link_across_the_floor() {
        let chf = floor(5);
        assert_eq!(chf.span_count(), 25);
        let center = chf.spans[chf.cell(2, 2).index as usize];
        assert!((0..4).all(|dir| center.connected(dir)));
        let corner = chf.spans[chf.cell(0, 0).index as usize];
        assert!(!corner.connected(0) && !corner.connected(3));
        assert!(corner.connected(1) && corner.connected(2));
    }

    #[test]
    fn erosion_removes_the_rim() {
        let mut chf = floor(9);
        chf.erode_walkable_area(2);
        let walkable =
            |chf: &CompactHeightfield, x, z| chf.areas[chf.cell(x, z).index as usize] != NULL_AREA;
        assert!(walkable(&chf, 4, 4));
        assert!(!walkable(&chf, 0, 4));
        assert!(!walkable(&chf, 1, 4));
        assert!(walkable(&chf, 3, 4));
    }

    #[test]
    fn carving_marks_only_inside() {
        let mut chf = floor(9);
        let square = [
            Vec2::new(3.0, 3.0),
            Vec2::new(6.0, 3.0),
            Vec2::new(6.0, 6.0),
            Vec2::new(3.0, 6.0),
        ];
        chf.mark_convex_area(&square, -1.0, 1.0, NULL_AREA);
        let area = |x, z| chf.areas[chf.cell(x, z).index as usize];
        assert_eq!(area(4, 4), NULL_AREA);
        assert_eq!(area(1, 1), WALKABLE_AREA);
        // Above the floor's height range nothing is carved.
        let mut chf = floor(9);
        chf.mark_convex_area(&square, 2.0, 3.0, NULL_AREA);
        assert_eq!(chf.areas[chf.cell(4, 4).index as usize], WALKABLE_AREA);
    }
}
