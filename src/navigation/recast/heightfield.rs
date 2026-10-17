//! The solid heightfield: triangles rasterized into columns of solid spans, then the
//! filters that decide where an agent can stand.
//!
//! Each column keeps every solid interval it holds, so floors above floors (a bridge
//! over a path, a balcony over a room) stay separate: that is what lets the navmesh
//! follow overhangs.

use glam::Vec3;

use super::{FORBIDDEN_AREA, NULL_AREA, SPAN_MAX_HEIGHT};

/// A solid interval in a column, in cell heights above the field's floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Span {
    pub smin: u16,
    pub smax: u16,
    /// What its top is: [`NULL_AREA`] (not walkable) or an area id.
    pub area: u8,
    /// The exact height of its top surface (m): `smax` rounds it up to a whole cell,
    /// which would leave agents floating up to a cell above the floor.
    pub top: f32,
    /// Its top is forbidden ground ([`FORBIDDEN_AREA`]): never made walkable.
    pub forbidden: bool,
}

/// Columns of solid spans over a `width` x `height` grid of cells (x by z).
pub(crate) struct Heightfield {
    pub width: i32,
    pub height: i32,
    pub bmin: Vec3,
    pub bmax: Vec3,
    /// Cell size (x and z) and cell height (y), in metres.
    pub cs: f32,
    pub ch: f32,
    /// Spans per column, sorted from the bottom up and never overlapping.
    pub columns: Vec<Vec<Span>>,
}

impl Heightfield {
    pub fn new(width: i32, height: i32, bmin: Vec3, bmax: Vec3, cs: f32, ch: f32) -> Self {
        Self {
            width,
            height,
            bmin,
            bmax,
            cs,
            ch,
            columns: vec![Vec::new(); (width.max(0) * height.max(0)) as usize],
        }
    }

    pub fn column(&self, x: i32, z: i32) -> &[Span] {
        &self.columns[(x + z * self.width) as usize]
    }

    /// Add a solid interval, merging it with the spans it touches. Where the tops of two
    /// merged spans are within `merge_threshold` cells, the higher area id wins.
    /// `area` [`FORBIDDEN_AREA`] adds a span whose top stays unwalkable.
    #[allow(clippy::too_many_arguments)]
    pub fn add_span(
        &mut self,
        x: i32,
        z: i32,
        smin: u16,
        smax: u16,
        area: u8,
        top: f32,
        merge_threshold: i32,
    ) {
        let column = &mut self.columns[(x + z * self.width) as usize];
        let forbidden = area == FORBIDDEN_AREA;
        let area = if forbidden { NULL_AREA } else { area };
        let mut new = Span {
            smin,
            smax,
            area,
            top,
            forbidden,
        };
        let mut index = 0;
        while index < column.len() {
            let current = column[index];
            if current.smin > new.smax {
                break;
            }
            if current.smax < new.smin {
                index += 1;
                continue;
            }
            new.smin = new.smin.min(current.smin);
            if current.smax > new.smax {
                new.top = current.top;
                new.forbidden = current.forbidden;
            } else if current.smax == new.smax {
                new.top = new.top.max(current.top);
                new.forbidden |= current.forbidden;
            }
            new.smax = new.smax.max(current.smax);
            if (i32::from(new.smax) - i32::from(current.smax)).abs() <= merge_threshold {
                new.area = new.area.max(current.area);
            }
            if new.forbidden {
                new.area = NULL_AREA;
            }
            column.remove(index);
        }
        column.insert(index, new);
    }

    /// Rasterize triangles (`areas` one per triangle) into the field.
    pub fn rasterize_triangles(
        &mut self,
        verts: &[Vec3],
        tris: &[[u32; 3]],
        areas: &[u8],
        merge_threshold: i32,
    ) {
        let ics = 1.0 / self.cs;
        let ich = 1.0 / self.ch;
        for (tri, &area) in tris.iter().zip(areas) {
            let v0 = verts[tri[0] as usize];
            let v1 = verts[tri[1] as usize];
            let v2 = verts[tri[2] as usize];
            self.rasterize_triangle(v0, v1, v2, area, ics, ich, merge_threshold);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn rasterize_triangle(
        &mut self,
        v0: Vec3,
        v1: Vec3,
        v2: Vec3,
        area: u8,
        ics: f32,
        ich: f32,
        merge_threshold: i32,
    ) {
        let tmin = v0.min(v1).min(v2);
        let tmax = v0.max(v1).max(v2);
        if tmin.x > self.bmax.x
            || tmax.x < self.bmin.x
            || tmin.y > self.bmax.y
            || tmax.y < self.bmin.y
            || tmin.z > self.bmax.z
            || tmax.z < self.bmin.z
        {
            return;
        }
        let w = self.width;
        let h = self.height;
        let by = self.bmax.y - self.bmin.y;
        let cs = self.cs;
        // The triangle's height over a point, for the exact floor height of each cell.
        let normal = (v1 - v0).cross(v2 - v0);
        let plane_height = |x: f32, z: f32| -> Option<f32> {
            (normal.y.abs() > 1e-6)
                .then(|| v0.y - (normal.x * (x - v0.x) + normal.z * (z - v0.z)) / normal.y)
        };

        // The rows the triangle covers (one before the grid, to clip that part away).
        let z0 = (((tmin.z - self.bmin.z) * ics) as i32).clamp(-1, h - 1);
        let z1 = (((tmax.z - self.bmin.z) * ics) as i32).clamp(0, h - 1);

        let mut rest: Vec<Vec3> = vec![v0, v1, v2];
        let mut row = Vec::with_capacity(7);
        let mut cell = Vec::with_capacity(7);
        let mut row_rest = Vec::with_capacity(7);
        let mut remainder = Vec::with_capacity(7);
        for z in z0..=z1 {
            let cz = self.bmin.z + z as f32 * cs;
            divide_poly(&rest, &mut row, &mut remainder, cz + cs, 2);
            std::mem::swap(&mut rest, &mut remainder);
            if row.len() < 3 || z < 0 {
                continue;
            }
            let (mut min_x, mut max_x) = (row[0].x, row[0].x);
            for v in &row[1..] {
                min_x = min_x.min(v.x);
                max_x = max_x.max(v.x);
            }
            let x0 = ((min_x - self.bmin.x) * ics) as i32;
            let x1 = ((max_x - self.bmin.x) * ics) as i32;
            if x1 < 0 || x0 >= w {
                continue;
            }
            let x0 = x0.clamp(-1, w - 1);
            let x1 = x1.clamp(0, w - 1);

            let mut row_poly = std::mem::take(&mut row);
            for x in x0..=x1 {
                let cx = self.bmin.x + x as f32 * cs;
                divide_poly(&row_poly, &mut cell, &mut row_rest, cx + cs, 0);
                std::mem::swap(&mut row_poly, &mut row_rest);
                if cell.len() < 3 || x < 0 {
                    continue;
                }
                let (mut smin, mut smax) = (cell[0].y, cell[0].y);
                for v in &cell[1..] {
                    smin = smin.min(v.y);
                    smax = smax.max(v.y);
                }
                smin -= self.bmin.y;
                smax -= self.bmin.y;
                if smax < 0.0 || smin > by {
                    continue;
                }
                let smin = smin.max(0.0);
                let smax = smax.min(by);
                let ismin = ((smin * ich).floor() as i32).clamp(0, SPAN_MAX_HEIGHT);
                let ismax = ((smax * ich).ceil() as i32).clamp(ismin + 1, SPAN_MAX_HEIGHT);
                let top = plane_height(cx + cs * 0.5, cz + cs * 0.5)
                    .map_or(self.bmin.y + smax, |y| {
                        y.clamp(self.bmin.y + smin, self.bmin.y + smax)
                    });
                self.add_span(x, z, ismin as u16, ismax as u16, area, top, merge_threshold);
            }
            row = row_poly;
        }
    }

    /// Let agents step onto low obstacles (curbs, stair treads seen from the side): a
    /// non-walkable span whose top is within `walkable_climb` of the walkable span below
    /// it takes that span's area.
    pub fn filter_low_hanging_obstacles(&mut self, walkable_climb: i32) {
        for column in &mut self.columns {
            let mut previous: Option<(u16, u8, bool)> = None;
            for span in column.iter_mut() {
                let walkable = span.area != NULL_AREA;
                if let Some((previous_max, previous_area, previous_walkable)) = previous {
                    if !walkable
                        && !span.forbidden
                        && previous_walkable
                        && (i32::from(span.smax) - i32::from(previous_max)).abs() <= walkable_climb
                    {
                        span.area = previous_area;
                    }
                }
                previous = Some((span.smax, span.area, walkable));
            }
        }
    }

    /// Spans at a ledge (a drop deeper than `walkable_climb` next to them) or on a slope
    /// too steep between neighbours are not walkable.
    pub fn filter_ledge_spans(&mut self, walkable_height: i32, walkable_climb: i32) {
        let w = self.width;
        let h = self.height;
        let mut unwalkable = Vec::new();
        for z in 0..h {
            for x in 0..w {
                let column = self.column(x, z);
                for (index, span) in column.iter().enumerate() {
                    if span.area == NULL_AREA {
                        continue;
                    }
                    let floor = i32::from(span.smax);
                    let ceiling = column
                        .get(index + 1)
                        .map_or(SPAN_MAX_HEIGHT, |next| i32::from(next.smin));
                    let mut lowest_difference = SPAN_MAX_HEIGHT;
                    let mut lowest_traversable = floor;
                    let mut highest_traversable = floor;
                    'dirs: for dir in 0..4 {
                        let nx = x + super::dir_offset_x(dir);
                        let nz = z + super::dir_offset_z(dir);
                        if nx < 0 || nz < 0 || nx >= w || nz >= h {
                            lowest_difference = -walkable_climb - 1;
                            break;
                        }
                        let neighbor = self.column(nx, nz);
                        // The space under the neighbour's lowest span is a drop to nowhere.
                        let neighbor_ceiling = neighbor
                            .first()
                            .map_or(SPAN_MAX_HEIGHT, |first| i32::from(first.smin));
                        if ceiling.min(neighbor_ceiling) - floor >= walkable_height {
                            lowest_difference = -walkable_climb - 1;
                            break 'dirs;
                        }
                        for (n, neighbor_span) in neighbor.iter().enumerate() {
                            let neighbor_floor = i32::from(neighbor_span.smax);
                            let neighbor_ceiling = neighbor
                                .get(n + 1)
                                .map_or(SPAN_MAX_HEIGHT, |next| i32::from(next.smin));
                            // Only neighbours an agent fits between.
                            if ceiling.min(neighbor_ceiling) - floor.max(neighbor_floor)
                                < walkable_height
                            {
                                continue;
                            }
                            let difference = neighbor_floor - floor;
                            lowest_difference = lowest_difference.min(difference);
                            if difference.abs() <= walkable_climb {
                                lowest_traversable = lowest_traversable.min(neighbor_floor);
                                highest_traversable = highest_traversable.max(neighbor_floor);
                            } else if difference < -walkable_climb {
                                break 'dirs;
                            }
                        }
                    }
                    if lowest_difference < -walkable_climb
                        || highest_traversable - lowest_traversable > walkable_climb
                    {
                        unwalkable.push(((x + z * w) as usize, index));
                    }
                }
            }
        }
        for (column, index) in unwalkable {
            self.columns[column][index].area = NULL_AREA;
        }
    }

    /// Spans without `walkable_height` of free space above them are not walkable.
    pub fn filter_low_height_spans(&mut self, walkable_height: i32) {
        for column in &mut self.columns {
            for index in 0..column.len() {
                let floor = i32::from(column[index].smax);
                let ceiling = column
                    .get(index + 1)
                    .map_or(SPAN_MAX_HEIGHT, |next| i32::from(next.smin));
                if ceiling - floor < walkable_height {
                    column[index].area = NULL_AREA;
                }
            }
        }
    }
}

/// Split the convex polygon `input` by the plane `axis = offset`: the part below it goes
/// to `below`, the rest to `above`.
fn divide_poly(
    input: &[Vec3],
    below: &mut Vec<Vec3>,
    above: &mut Vec<Vec3>,
    offset: f32,
    axis: usize,
) {
    below.clear();
    above.clear();
    if input.is_empty() {
        return;
    }
    let delta = |v: &Vec3| offset - v[axis];
    let mut b = input.len() - 1;
    for a in 0..input.len() {
        let da = delta(&input[a]);
        let db = delta(&input[b]);
        let same_side = (da >= 0.0) == (db >= 0.0);
        if !same_side {
            let s = db / (db - da);
            let point = input[b] + (input[a] - input[b]) * s;
            below.push(point);
            above.push(point);
            if da > 0.0 {
                below.push(input[a]);
            } else if da < 0.0 {
                above.push(input[a]);
            }
        } else {
            if da >= 0.0 {
                below.push(input[a]);
                if da != 0.0 {
                    b = a;
                    continue;
                }
            }
            above.push(input[a]);
        }
        b = a;
    }
}

#[cfg(test)]
mod tests {
    use super::super::WALKABLE_AREA;
    use super::*;

    fn field(size: i32) -> Heightfield {
        Heightfield::new(
            size,
            size,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(size as f32, 10.0, size as f32),
            1.0,
            0.1,
        )
    }

    #[test]
    fn spans_merge_and_keep_order() {
        let mut hf = field(1);
        hf.add_span(0, 0, 10, 20, WALKABLE_AREA, 2.0, 1);
        hf.add_span(0, 0, 0, 5, NULL_AREA, 0.5, 1);
        hf.add_span(0, 0, 30, 40, WALKABLE_AREA, 4.0, 1);
        assert_eq!(hf.column(0, 0).len(), 3);
        // Overlapping the first two merges them; the top keeps the higher area.
        hf.add_span(0, 0, 4, 12, NULL_AREA, 1.2, 1);
        let column = hf.column(0, 0);
        assert_eq!(column.len(), 2);
        assert_eq!((column[0].smin, column[0].smax), (0, 20));
        assert_eq!(column[0].area, WALKABLE_AREA);
        assert_eq!(column[0].top, 2.0);
    }

    #[test]
    fn a_floor_quad_fills_every_cell_once() {
        let mut hf = field(4);
        let verts = [
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(4.0, 1.0, 0.0),
            Vec3::new(4.0, 1.0, 4.0),
            Vec3::new(0.0, 1.0, 4.0),
        ];
        hf.rasterize_triangles(&verts, &[[0, 2, 1], [0, 3, 2]], &[WALKABLE_AREA; 2], 1);
        for z in 0..4 {
            for x in 0..4 {
                let column = hf.column(x, z);
                assert_eq!(column.len(), 1, "cell {x},{z}");
                // 1 m in 0.1 m cells, rounded up (f32 puts it just above 10), and
                // exactly 1 m as the top's height.
                assert!((10..=11).contains(&column[0].smax));
                assert!((column[0].top - 1.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn dividing_a_triangle_keeps_its_area() {
        let tri = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0),
        ];
        let (mut below, mut above) = (Vec::new(), Vec::new());
        divide_poly(&tri, &mut below, &mut above, 1.0, 0);
        let area = |poly: &[Vec3]| {
            let mut sum = 0.0;
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                sum += a.x * b.z - b.x * a.z;
            }
            sum.abs() * 0.5
        };
        assert!((area(&below) + area(&above) - 2.0).abs() < 1e-5);
        assert!(below.iter().all(|v| v.x <= 1.0 + 1e-6));
    }
}
