//! Partitioning the walkable area into regions (watershed over the distance to the
//! nearest edge), so each region traces to a simple outline without holes.

use super::compact::CompactHeightfield;
use super::{dir_offset_x, dir_offset_z, BORDER_REG, NULL_AREA};

impl CompactHeightfield {
    /// The distance (in half cells, chamfered) of every span from the nearest edge of
    /// its area, smoothed; feeds [`Self::build_regions`].
    pub fn build_distance_field(&mut self) {
        let mut src = vec![0u16; self.spans.len()];
        let max = self.calculate_distance_field(&mut src);
        self.max_distance = max;
        self.dist = self.box_blur(1, &src);
    }

    fn calculate_distance_field(&self, src: &mut [u16]) -> u16 {
        let w = self.width;
        let h = self.height;
        src.fill(0xffff);
        for z in 0..h {
            for x in 0..w {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    let span = self.spans[i];
                    let area = self.areas[i];
                    let same = (0..4)
                        .filter(|&dir| {
                            span.connected(dir)
                                && self.areas[self.neighbor_index(x, z, &span, dir)] == area
                        })
                        .count();
                    if same != 4 {
                        src[i] = 0;
                    }
                }
            }
        }
        let relax = |src: &mut [u16], i: usize, j: usize, cost: u16| {
            let nd = src[j].saturating_add(cost);
            if nd < src[i] {
                src[i] = nd;
            }
        };
        for z in 0..h {
            for x in 0..w {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    let span = self.spans[i];
                    if span.connected(0) {
                        let ai = self.neighbor_index(x, z, &span, 0);
                        relax(src, i, ai, 2);
                        let a = self.spans[ai];
                        if a.connected(3) {
                            let aai = self.neighbor_index(x + dir_offset_x(0), z, &a, 3);
                            relax(src, i, aai, 3);
                        }
                    }
                    if span.connected(3) {
                        let ai = self.neighbor_index(x, z, &span, 3);
                        relax(src, i, ai, 2);
                        let a = self.spans[ai];
                        if a.connected(2) {
                            let aai = self.neighbor_index(x, z + dir_offset_z(3), &a, 2);
                            relax(src, i, aai, 3);
                        }
                    }
                }
            }
        }
        for z in (0..h).rev() {
            for x in (0..w).rev() {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    let span = self.spans[i];
                    if span.connected(2) {
                        let ai = self.neighbor_index(x, z, &span, 2);
                        relax(src, i, ai, 2);
                        let a = self.spans[ai];
                        if a.connected(1) {
                            let aai = self.neighbor_index(x + dir_offset_x(2), z, &a, 1);
                            relax(src, i, aai, 3);
                        }
                    }
                    if span.connected(1) {
                        let ai = self.neighbor_index(x, z, &span, 1);
                        relax(src, i, ai, 2);
                        let a = self.spans[ai];
                        if a.connected(0) {
                            let aai = self.neighbor_index(x, z + dir_offset_z(1), &a, 0);
                            relax(src, i, aai, 3);
                        }
                    }
                }
            }
        }
        src.iter().copied().max().unwrap_or(0)
    }

    fn box_blur(&self, thr: u16, src: &[u16]) -> Vec<u16> {
        let w = self.width;
        let h = self.height;
        let thr = thr * 2;
        let mut dst = vec![0u16; src.len()];
        for z in 0..h {
            for x in 0..w {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    let span = self.spans[i];
                    let cd = u32::from(src[i]);
                    if cd <= u32::from(thr) {
                        dst[i] = cd as u16;
                        continue;
                    }
                    let mut d = cd;
                    for dir in 0..4 {
                        if span.connected(dir) {
                            let ai = self.neighbor_index(x, z, &span, dir);
                            d += u32::from(src[ai]);
                            let a = self.spans[ai];
                            let dir2 = (dir + 1) & 3;
                            if a.connected(dir2) {
                                let aai = self.neighbor_index(
                                    x + dir_offset_x(dir),
                                    z + dir_offset_z(dir),
                                    &a,
                                    dir2,
                                );
                                d += u32::from(src[aai]);
                            } else {
                                d += cd;
                            }
                        } else {
                            d += cd * 2;
                        }
                    }
                    dst[i] = ((d + 5) / 9) as u16;
                }
            }
        }
        dst
    }

    /// Watershed partitioning: grow regions from the spans furthest from any edge
    /// downward, then merge and drop small ones. `border_size` cells along each side
    /// (the tile's border) become border regions that are traced but never kept.
    pub fn build_regions(
        &mut self,
        border_size: i32,
        min_region_area: i32,
        merge_region_area: i32,
    ) {
        let w = self.width;
        let h = self.height;
        let count = self.spans.len();
        let mut src_reg = vec![0u16; count];
        let mut src_dist = vec![0u16; count];
        let mut region_id: u16 = 1;
        let mut level = (self.max_distance + 1) & !1;
        const EXPAND_ITERS: i32 = 8;

        if border_size > 0 {
            let bw = w.min(border_size);
            let bh = h.min(border_size);
            self.paint_rect_region(0, bw, 0, h, region_id | BORDER_REG, &mut src_reg);
            region_id += 1;
            self.paint_rect_region(w - bw, w, 0, h, region_id | BORDER_REG, &mut src_reg);
            region_id += 1;
            self.paint_rect_region(0, w, 0, bh, region_id | BORDER_REG, &mut src_reg);
            region_id += 1;
            self.paint_rect_region(0, w, h - bh, h, region_id | BORDER_REG, &mut src_reg);
            region_id += 1;
        }
        self.border_size = border_size;

        const NB_STACKS: usize = 8;
        let mut lvl_stacks: Vec<Vec<LevelEntry>> = vec![Vec::new(); NB_STACKS];
        let mut stack: Vec<LevelEntry> = Vec::new();
        let mut s_id: isize = -1;
        while level > 0 {
            level = level.saturating_sub(2);
            s_id = (s_id + 1) & (NB_STACKS as isize - 1);
            let sid = s_id as usize;
            if sid == 0 {
                self.sort_cells_by_level(level, &src_reg, &mut lvl_stacks, 1);
            } else {
                let (before, after) = lvl_stacks.split_at_mut(sid);
                append_stacks(&before[sid - 1], &mut after[0], &src_reg);
            }
            self.expand_regions(
                EXPAND_ITERS,
                level,
                &mut src_reg,
                &mut src_dist,
                &mut lvl_stacks[sid],
                false,
            );
            let entries = std::mem::take(&mut lvl_stacks[sid]);
            for entry in &entries {
                if entry.index >= 0
                    && src_reg[entry.index as usize] == 0
                    && self.flood_region(
                        entry.x,
                        entry.z,
                        entry.index as usize,
                        level,
                        region_id,
                        &mut src_reg,
                        &mut src_dist,
                        &mut stack,
                    )
                {
                    if region_id == 0xffff {
                        log::warn!("navmesh: region id overflow");
                        return;
                    }
                    region_id += 1;
                }
            }
            lvl_stacks[sid] = entries;
        }
        self.expand_regions(
            EXPAND_ITERS * 8,
            0,
            &mut src_reg,
            &mut src_dist,
            &mut stack,
            true,
        );

        self.max_regions = region_id;
        let mut max_region = region_id;
        merge_and_filter_regions(
            self,
            min_region_area,
            merge_region_area,
            &mut max_region,
            &mut src_reg,
        );
        self.max_regions = max_region;
        for (span, reg) in self.spans.iter_mut().zip(&src_reg) {
            span.reg = *reg;
        }
    }

    fn paint_rect_region(
        &self,
        min_x: i32,
        max_x: i32,
        min_z: i32,
        max_z: i32,
        reg: u16,
        src_reg: &mut [u16],
    ) {
        for z in min_z..max_z {
            for x in min_x..max_x {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    if self.areas[i as usize] != NULL_AREA {
                        src_reg[i as usize] = reg;
                    }
                }
            }
        }
    }

    fn sort_cells_by_level(
        &self,
        start_level: u16,
        src_reg: &[u16],
        stacks: &mut [Vec<LevelEntry>],
        log_levels_per_stack: u32,
    ) {
        let start_level = i32::from(start_level >> log_levels_per_stack);
        for stack in stacks.iter_mut() {
            stack.clear();
        }
        for z in 0..self.height {
            for x in 0..self.width {
                let cell = self.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    if self.areas[i] == NULL_AREA || src_reg[i] != 0 {
                        continue;
                    }
                    let level = i32::from(self.dist[i] >> log_levels_per_stack);
                    let s_id = (start_level - level).max(0) as usize;
                    if s_id >= stacks.len() {
                        continue;
                    }
                    stacks[s_id].push(LevelEntry {
                        x,
                        z,
                        index: i as i32,
                    });
                }
            }
        }
    }

    fn expand_regions(
        &self,
        max_iter: i32,
        level: u16,
        src_reg: &mut [u16],
        src_dist: &mut [u16],
        stack: &mut Vec<LevelEntry>,
        fill_stack: bool,
    ) {
        if fill_stack {
            stack.clear();
            for z in 0..self.height {
                for x in 0..self.width {
                    let cell = self.cell(x, z);
                    for i in cell.index..cell.index + cell.count {
                        let i = i as usize;
                        if self.dist[i] >= level && src_reg[i] == 0 && self.areas[i] != NULL_AREA {
                            stack.push(LevelEntry {
                                x,
                                z,
                                index: i as i32,
                            });
                        }
                    }
                }
            }
        } else {
            for entry in stack.iter_mut() {
                if entry.index >= 0 && src_reg[entry.index as usize] != 0 {
                    entry.index = -1;
                }
            }
        }

        let mut dirty: Vec<(usize, u16, u16)> = Vec::new();
        let mut iter = 0;
        while !stack.is_empty() {
            let mut failed = 0;
            dirty.clear();
            for entry in stack.iter_mut() {
                if entry.index < 0 {
                    failed += 1;
                    continue;
                }
                let i = entry.index as usize;
                let (x, z) = (entry.x, entry.z);
                let mut r = src_reg[i];
                let mut d2 = 0xffffu16;
                let area = self.areas[i];
                let span = self.spans[i];
                for dir in 0..4 {
                    if !span.connected(dir) {
                        continue;
                    }
                    let ai = self.neighbor_index(x, z, &span, dir);
                    if self.areas[ai] != area {
                        continue;
                    }
                    if src_reg[ai] > 0
                        && (src_reg[ai] & BORDER_REG) == 0
                        && u32::from(src_dist[ai]) + 2 < u32::from(d2)
                    {
                        r = src_reg[ai];
                        d2 = src_dist[ai] + 2;
                    }
                }
                if r != 0 {
                    entry.index = -1;
                    dirty.push((i, r, d2));
                } else {
                    failed += 1;
                }
            }
            for &(i, r, d2) in &dirty {
                src_reg[i] = r;
                src_dist[i] = d2;
            }
            if failed == stack.len() {
                break;
            }
            if level > 0 {
                iter += 1;
                if iter >= max_iter {
                    break;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn flood_region(
        &self,
        x: i32,
        z: i32,
        i: usize,
        level: u16,
        r: u16,
        src_reg: &mut [u16],
        src_dist: &mut [u16],
        stack: &mut Vec<LevelEntry>,
    ) -> bool {
        let area = self.areas[i];
        stack.clear();
        stack.push(LevelEntry {
            x,
            z,
            index: i as i32,
        });
        src_reg[i] = r;
        src_dist[i] = 0;
        let lev = level.saturating_sub(2);
        let mut count = 0;
        while let Some(entry) = stack.pop() {
            let (cx, cz, ci) = (entry.x, entry.z, entry.index as usize);
            let cs = self.spans[ci];
            // Stop at spans next to another region (they belong to whichever grows there).
            let mut ar = 0u16;
            for dir in 0..4 {
                if !cs.connected(dir) {
                    continue;
                }
                let ax = cx + dir_offset_x(dir);
                let az = cz + dir_offset_z(dir);
                let ai = self.neighbor_index(cx, cz, &cs, dir);
                if self.areas[ai] != area {
                    continue;
                }
                let nr = src_reg[ai];
                if nr & BORDER_REG != 0 {
                    continue;
                }
                if nr != 0 && nr != r {
                    ar = nr;
                    break;
                }
                let a = self.spans[ai];
                let dir2 = (dir + 1) & 3;
                if a.connected(dir2) {
                    let ai2 = self.neighbor_index(ax, az, &a, dir2);
                    if self.areas[ai2] != area {
                        continue;
                    }
                    let nr2 = src_reg[ai2];
                    if nr2 != 0 && nr2 != r {
                        ar = nr2;
                        break;
                    }
                }
            }
            if ar != 0 {
                src_reg[ci] = 0;
                continue;
            }
            count += 1;
            for dir in 0..4 {
                if !cs.connected(dir) {
                    continue;
                }
                let ax = cx + dir_offset_x(dir);
                let az = cz + dir_offset_z(dir);
                let ai = self.neighbor_index(cx, cz, &cs, dir);
                if self.areas[ai] != area {
                    continue;
                }
                if self.dist[ai] >= lev && src_reg[ai] == 0 {
                    src_reg[ai] = r;
                    src_dist[ai] = 0;
                    stack.push(LevelEntry {
                        x: ax,
                        z: az,
                        index: ai as i32,
                    });
                }
            }
        }
        count > 0
    }

    /// Whether the edge of span `i` toward `dir` leaves its region.
    fn is_solid_edge(&self, src_reg: &[u16], x: i32, z: i32, i: usize, dir: usize) -> bool {
        let span = self.spans[i];
        let mut r = 0;
        if span.connected(dir) {
            r = src_reg[self.neighbor_index(x, z, &span, dir)];
        }
        r != src_reg[i]
    }

    /// The regions around a region's outline, in order (0 for unwalkable space).
    fn walk_region_contour(
        &self,
        mut x: i32,
        mut z: i32,
        mut i: usize,
        mut dir: usize,
        src_reg: &[u16],
        cont: &mut Vec<i32>,
    ) {
        let start_dir = dir;
        let start_i = i;
        let span = self.spans[i];
        let mut cur_reg = 0i32;
        if span.connected(dir) {
            cur_reg = i32::from(src_reg[self.neighbor_index(x, z, &span, dir)]);
        }
        cont.push(cur_reg);
        let mut iter = 0;
        loop {
            iter += 1;
            if iter >= 40000 {
                break;
            }
            let span = self.spans[i];
            if self.is_solid_edge(src_reg, x, z, i, dir) {
                let mut r = 0i32;
                if span.connected(dir) {
                    r = i32::from(src_reg[self.neighbor_index(x, z, &span, dir)]);
                }
                if r != cur_reg {
                    cur_reg = r;
                    cont.push(cur_reg);
                }
                dir = (dir + 1) & 3;
            } else {
                if !span.connected(dir) {
                    return;
                }
                let ni = self.neighbor_index(x, z, &span, dir);
                x += dir_offset_x(dir);
                z += dir_offset_z(dir);
                i = ni;
                dir = (dir + 3) & 3;
            }
            if start_i == i && start_dir == dir {
                break;
            }
        }
        remove_adjacent_duplicates(cont);
    }
}

#[derive(Debug, Clone, Copy)]
struct LevelEntry {
    x: i32,
    z: i32,
    /// Span index, or -1 once taken.
    index: i32,
}

fn append_stacks(src: &[LevelEntry], dst: &mut Vec<LevelEntry>, src_reg: &[u16]) {
    for entry in src {
        if entry.index < 0 || src_reg[entry.index as usize] != 0 {
            continue;
        }
        dst.push(*entry);
    }
}

fn remove_adjacent_duplicates(list: &mut Vec<i32>) {
    if list.len() <= 1 {
        return;
    }
    let mut j = 0;
    while j < list.len() && list.len() > 1 {
        let nj = (j + 1) % list.len();
        if list[j] == list[nj] {
            list.remove(j);
        } else {
            j += 1;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Region {
    span_count: i32,
    id: u16,
    area_type: u8,
    remap: bool,
    visited: bool,
    overlap: bool,
    connections: Vec<i32>,
    floors: Vec<i32>,
}

fn add_unique_floor_region(reg: &mut Region, n: i32) {
    if !reg.floors.contains(&n) {
        reg.floors.push(n);
    }
}

fn can_merge_with_region(a: &Region, b: &Region) -> bool {
    if a.area_type != b.area_type {
        return false;
    }
    let shared = a
        .connections
        .iter()
        .filter(|&&c| c == i32::from(b.id))
        .count();
    if shared > 1 {
        return false;
    }
    !a.floors.contains(&i32::from(b.id))
}

/// Merge region `b` into `a` (both by index); false when they do not share an edge.
fn merge_regions(regions: &mut [Region], a: usize, b: usize) -> bool {
    let aid = i32::from(regions[a].id);
    let bid = i32::from(regions[b].id);
    let acon = regions[a].connections.clone();
    let bcon = regions[b].connections.clone();
    let Some(insa) = acon.iter().position(|&c| c == bid) else {
        return false;
    };
    let Some(insb) = bcon.iter().position(|&c| c == aid) else {
        return false;
    };
    let mut merged = Vec::with_capacity(acon.len() + bcon.len());
    for k in 0..acon.len().saturating_sub(1) {
        merged.push(acon[(insa + 1 + k) % acon.len()]);
    }
    for k in 0..bcon.len().saturating_sub(1) {
        merged.push(bcon[(insb + 1 + k) % bcon.len()]);
    }
    remove_adjacent_duplicates(&mut merged);
    let b_floors = regions[b].floors.clone();
    let b_spans = regions[b].span_count;
    let ra = &mut regions[a];
    ra.connections = merged;
    for floor in b_floors {
        add_unique_floor_region(ra, floor);
    }
    ra.span_count += b_spans;
    let rb = &mut regions[b];
    rb.span_count = 0;
    rb.connections.clear();
    true
}

fn replace_neighbour(reg: &mut Region, old_id: i32, new_id: i32) {
    let mut changed = false;
    for c in reg.connections.iter_mut() {
        if *c == old_id {
            *c = new_id;
            changed = true;
        }
    }
    for f in reg.floors.iter_mut() {
        if *f == old_id {
            *f = new_id;
        }
    }
    if changed {
        remove_adjacent_duplicates(&mut reg.connections);
    }
}

fn is_region_connected_to_border(reg: &Region) -> bool {
    reg.connections.contains(&0)
}

fn merge_and_filter_regions(
    chf: &CompactHeightfield,
    min_region_area: i32,
    merge_region_size: i32,
    max_region_id: &mut u16,
    src_reg: &mut [u16],
) {
    let w = chf.width;
    let h = chf.height;
    let nreg = usize::from(*max_region_id) + 1;
    let mut regions: Vec<Region> = (0..nreg)
        .map(|i| Region {
            id: i as u16,
            ..Default::default()
        })
        .collect();

    // Each region's size, the regions under or over it, and those around its outline.
    for z in 0..h {
        for x in 0..w {
            let cell = chf.cell(x, z);
            for i in cell.index..cell.index + cell.count {
                let i = i as usize;
                let r = usize::from(src_reg[i]);
                if r == 0 || r >= nreg {
                    continue;
                }
                regions[r].span_count += 1;
                for j in cell.index..cell.index + cell.count {
                    let j = j as usize;
                    if i == j {
                        continue;
                    }
                    let floor_id = usize::from(src_reg[j]);
                    if floor_id == 0 || floor_id >= nreg {
                        continue;
                    }
                    if floor_id == r {
                        regions[r].overlap = true;
                    }
                    add_unique_floor_region(&mut regions[r], floor_id as i32);
                }
                if !regions[r].connections.is_empty() {
                    continue;
                }
                regions[r].area_type = chf.areas[i];
                let edge_dir = (0..4).find(|&dir| chf.is_solid_edge(src_reg, x, z, i, dir));
                if let Some(dir) = edge_dir {
                    let mut connections = Vec::new();
                    chf.walk_region_contour(x, z, i, dir, src_reg, &mut connections);
                    regions[r].connections = connections;
                }
            }
        }
    }

    // Remove groups of connected regions that are too small, unless they reach the
    // tile border (their size there cannot be judged).
    let mut stack: Vec<usize> = Vec::new();
    let mut trace: Vec<usize> = Vec::new();
    for i in 0..nreg {
        {
            let reg = &regions[i];
            if reg.id == 0 || reg.id & BORDER_REG != 0 || reg.span_count == 0 || reg.visited {
                continue;
            }
        }
        let mut connects_to_border = false;
        let mut span_count = 0;
        stack.clear();
        trace.clear();
        regions[i].visited = true;
        stack.push(i);
        while let Some(ri) = stack.pop() {
            span_count += regions[ri].span_count;
            trace.push(ri);
            let connections = regions[ri].connections.clone();
            for c in connections {
                if c & i32::from(BORDER_REG) != 0 {
                    connects_to_border = true;
                    continue;
                }
                let Some(neighbour) = regions.get_mut(c as usize) else {
                    continue;
                };
                if neighbour.visited || neighbour.id == 0 || neighbour.id & BORDER_REG != 0 {
                    continue;
                }
                stack.push(usize::from(neighbour.id));
                neighbour.visited = true;
            }
        }
        if span_count < min_region_area && !connects_to_border {
            for &t in &trace {
                regions[t].span_count = 0;
                regions[t].id = 0;
            }
        }
    }

    // Merge small regions into their smallest neighbour.
    loop {
        let mut merge_count = 0;
        for i in 0..nreg {
            let (id, overlap, span_count) = {
                let reg = &regions[i];
                (reg.id, reg.overlap, reg.span_count)
            };
            if id == 0 || id & BORDER_REG != 0 || overlap || span_count == 0 {
                continue;
            }
            if span_count > merge_region_size && is_region_connected_to_border(&regions[i]) {
                continue;
            }
            let mut smallest = i32::MAX;
            let mut merge_id = id;
            for &c in &regions[i].connections {
                if c & i32::from(BORDER_REG) != 0 {
                    continue;
                }
                let Some(other) = regions.get(c as usize) else {
                    continue;
                };
                if other.id == 0 || other.id & BORDER_REG != 0 || other.overlap {
                    continue;
                }
                if other.span_count < smallest
                    && can_merge_with_region(&regions[i], other)
                    && can_merge_with_region(other, &regions[i])
                {
                    smallest = other.span_count;
                    merge_id = other.id;
                }
            }
            if merge_id != id {
                let old_id = i32::from(id);
                let target = usize::from(merge_id);
                if merge_regions(&mut regions, target, i) {
                    for reg in regions.iter_mut() {
                        if reg.id == 0 || reg.id & BORDER_REG != 0 {
                            continue;
                        }
                        if i32::from(reg.id) == old_id {
                            reg.id = merge_id;
                        }
                        replace_neighbour(reg, old_id, i32::from(merge_id));
                    }
                    merge_count += 1;
                }
            }
        }
        if merge_count == 0 {
            break;
        }
    }

    // Compact the ids.
    for reg in regions.iter_mut() {
        reg.remap = reg.id != 0 && reg.id & BORDER_REG == 0;
    }
    let mut next_id: u16 = 0;
    for i in 0..nreg {
        if !regions[i].remap {
            continue;
        }
        let old_id = regions[i].id;
        next_id += 1;
        for reg in regions[i..].iter_mut() {
            if reg.id == old_id {
                reg.id = next_id;
                reg.remap = false;
            }
        }
    }
    *max_region_id = next_id;
    for reg in src_reg.iter_mut() {
        if *reg & BORDER_REG == 0 {
            *reg = regions[usize::from(*reg)].id;
        }
    }
}
