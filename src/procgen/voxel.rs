//! Voxel models and greedy meshing.
//!
//! A [`VoxelGrid`] holds one byte per cell: 0 is empty, 1..=255 picks an entry of the
//! model's palette. [`VoxelModel::build`] meshes it into one [`GeneratedMesh`] per palette
//! entry (or per group of entries, see [`VoxelLook::parts`]), so each colour can draw with
//! its own material:
//!
//! - only faces between a solid cell and an empty one (or the outside) are emitted, so the
//!   inside of a model and the faces where two colours touch cost nothing;
//! - coplanar faces of one colour are merged into as few rectangles as a greedy sweep
//!   finds, so a flat 10x4 wall is one quad, not forty;
//! - each vertex gets a colour: the key's colour from [`VoxelLook::colors`] times the
//!   ambient occlusion baked from the cells around that corner, so creases and inner
//!   corners darken. Only faces with the same corner shading merge, which keeps the
//!   shading exact.
//!
//! Grid coordinates: x to the right (+X), y up (+Y), z toward the viewer (+Z). The cell
//! (x, y, z) covers `origin + [x, x+1] * voxel_size` and so on, where the origin puts the
//! model's pivot at the local origin.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use glam::Vec3;

use super::GeneratedMesh;
use crate::asset::MeshData;
use crate::renderer::Vertex;

/// Largest grid along any axis.
pub const MAX_DIMENSION: u32 = 1024;
/// Most cells in one grid (16M, 16 MB).
pub const MAX_CELLS: usize = 1 << 24;
/// Most distinct palette entries in one model.
pub const MAX_KEYS: usize = 255;

/// A palette entry as scripts and remote calls name it: a string (a colour name, a text-art
/// character) or an integer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VoxelKey {
    Int(i64),
    Str(String),
}

impl std::fmt::Display for VoxelKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VoxelKey::Int(value) => write!(f, "{value}"),
            VoxelKey::Str(value) => f.write_str(value),
        }
    }
}

/// Dense grid of palette indices (0 = empty).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VoxelGrid {
    size: [u32; 3],
    cells: Vec<u8>,
}

impl VoxelGrid {
    pub fn new(size: [u32; 3]) -> Result<Self, String> {
        if size.iter().any(|&s| s == 0 || s > MAX_DIMENSION) {
            return Err(format!(
                "voxel grid size {size:?} must be 1..{MAX_DIMENSION} along each axis"
            ));
        }
        let count = size.iter().map(|&s| s as usize).product::<usize>();
        if count > MAX_CELLS {
            return Err(format!(
                "voxel grid {size:?} has {count} cells; the limit is {MAX_CELLS}"
            ));
        }
        Ok(Self {
            size,
            cells: vec![0; count],
        })
    }

    pub fn size(&self) -> [u32; 3] {
        self.size
    }

    fn index(&self, x: u32, y: u32, z: u32) -> usize {
        let [w, h, _] = self.size;
        x as usize + w as usize * (y as usize + h as usize * z as usize)
    }

    pub fn get(&self, x: u32, y: u32, z: u32) -> u8 {
        self.cells[self.index(x, y, z)]
    }

    pub fn set(&mut self, x: u32, y: u32, z: u32, value: u8) {
        let index = self.index(x, y, z);
        self.cells[index] = value;
    }

    /// Solid cells.
    pub fn solid_count(&self) -> usize {
        self.cells.iter().filter(|&&cell| cell != 0).count()
    }

    /// Faces a mesh with one cube per solid cell would draw (6 per cell).
    pub fn naive_quads(&self) -> usize {
        self.solid_count() * 6
    }

    /// Whether the cell at `pos` (which may lie outside the grid) is solid.
    fn solid_at(&self, pos: [isize; 3]) -> bool {
        let dims = self.size;
        if (0..3).any(|axis| pos[axis] < 0 || pos[axis] >= dims[axis] as isize) {
            return false;
        }
        self.cells[self.flat(pos.map(|p| p as usize))] != 0
    }

    /// Ambient occlusion of the four corners of the face of cell `pos` that looks along
    /// `d` into the empty cell `front`: 3 is open, 0 is a corner closed on both sides.
    /// Corners in quad order: (0,0), (1,0), (1,1), (0,1) along axes `a`, `b`. Packed two
    /// bits each.
    fn face_ao(&self, front: [usize; 3], front_k: Option<usize>, d: usize) -> u8 {
        let (a, b) = ((d + 1) % 3, (d + 2) % 3);
        // A face on the grid's outer wall looks at nothing.
        let Some(front_k) = front_k else {
            return 0xff;
        };
        let mut base = front.map(|p| p as isize);
        base[d] = front_k as isize;
        let at = |da: isize, db: isize| {
            let mut p = base;
            p[a] += da;
            p[b] += db;
            self.solid_at(p)
        };
        let mut packed = 0u8;
        for (corner, (sa, sb)) in [(-1, -1), (1, -1), (1, 1), (-1, 1)].into_iter().enumerate() {
            let side_a = at(sa, 0);
            let side_b = at(0, sb);
            let level = if side_a && side_b {
                0
            } else {
                3 - side_a as u8 - side_b as u8 - at(sa, sb) as u8
            };
            packed |= level << (corner * 2);
        }
        packed
    }

    /// Greedy-mesh the grid into the parts `plan` names. `origin` is where the grid corner
    /// (0, 0, 0) lands.
    fn mesh_parts(&self, voxel_size: Vec3, origin: Vec3, plan: &MeshPlan) -> Vec<PartBuilder> {
        let mut parts: Vec<PartBuilder> = (0..plan.part_count)
            .map(|_| PartBuilder::default())
            .collect();
        let dims = self.size.map(|s| s as usize);
        // Low byte: palette value (0 = no face); high byte: packed corner occlusion.
        let mut mask: Vec<u16> = Vec::new();
        let mut pos = [0usize; 3];

        for d in 0..3 {
            let (a, b) = ((d + 1) % 3, (d + 2) % 3);
            let (len_a, len_b) = (dims[a], dims[b]);
            mask.clear();
            mask.resize(len_a * len_b, 0);
            for positive in [true, false] {
                for k in 0..dims[d] {
                    // The faces of slice k that look along +d (or -d) at an empty cell.
                    let neighbor_k = if positive {
                        (k + 1 < dims[d]).then_some(k + 1)
                    } else {
                        k.checked_sub(1)
                    };
                    for j in 0..len_b {
                        for i in 0..len_a {
                            pos[d] = k;
                            pos[a] = i;
                            pos[b] = j;
                            let value = self.cells[self.flat(pos)];
                            let open = value != 0
                                && neighbor_k.is_none_or(|nk| {
                                    pos[d] = nk;
                                    self.cells[self.flat(pos)] == 0
                                });
                            mask[i + j * len_a] = if !open {
                                0
                            } else if plan.ao {
                                value as u16 | (self.face_ao(pos, neighbor_k, d) as u16) << 8
                            } else {
                                value as u16 | 0xff00
                            };
                        }
                    }

                    // Cover the mask with rectangles of one value and shading, row by row.
                    let plane = if positive { k + 1 } else { k };
                    for j in 0..len_b {
                        let mut i = 0;
                        while i < len_a {
                            let cell = mask[i + j * len_a];
                            if cell == 0 {
                                i += 1;
                                continue;
                            }
                            let mut w = 1;
                            while i + w < len_a && mask[i + w + j * len_a] == cell {
                                w += 1;
                            }
                            let mut h = 1;
                            'grow: while j + h < len_b {
                                let row = (j + h) * len_a;
                                if mask[row + i..row + i + w].iter().any(|&m| m != cell) {
                                    break 'grow;
                                }
                                h += 1;
                            }
                            for row in j..j + h {
                                mask[row * len_a + i..row * len_a + i + w].fill(0);
                            }
                            let value = (cell & 0xff) as usize;
                            let ao = (cell >> 8) as u8;
                            let shade =
                                [0, 1, 2, 3].map(|c| plan.shade[((ao >> (c * 2)) & 3) as usize]);
                            parts[plan.part_of[value]].quad(
                                Face {
                                    d,
                                    positive,
                                    plane,
                                    i,
                                    j,
                                    w,
                                    h,
                                },
                                voxel_size,
                                origin,
                                plan.colors[value],
                                shade,
                            );
                            i += w;
                        }
                    }
                }
            }
        }
        parts
    }

    fn flat(&self, pos: [usize; 3]) -> usize {
        let [w, h, _] = self.size;
        pos[0] + w as usize * (pos[1] + h as usize * pos[2])
    }
}

/// Brightness of a corner per occlusion level (0 = closed on both sides .. 3 = open) at
/// full strength.
const AO_LEVELS: [f32; 4] = [0.38, 0.6, 0.8, 1.0];

/// What the mesher needs per palette value, resolved from a [`VoxelLook`].
struct MeshPlan {
    part_count: usize,
    part_of: [usize; 256],
    colors: [[f32; 4]; 256],
    ao: bool,
    shade: [f32; 4],
}

/// How a model's cells are shaded and grouped into meshes. The default bakes ambient
/// occlusion, leaves every vertex white (the material colour shows as it is) and gives
/// each key its own mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct VoxelLook {
    /// Strength of the baked ambient occlusion: 0 turns it off, 1 is full.
    pub ao: f32,
    /// Vertex colour per key; it multiplies the material colour. Keys not listed are
    /// white.
    pub colors: HashMap<VoxelKey, [f32; 4]>,
    /// Which mesh part each key goes into. Keys that name the same part share one mesh
    /// (and so one material), with their own vertex colours. A key not listed is its own
    /// part (or goes into `part`).
    pub parts: HashMap<VoxelKey, VoxelKey>,
    /// The part of every key `parts` does not list; `None` gives each its own.
    pub part: Option<VoxelKey>,
}

impl Default for VoxelLook {
    fn default() -> Self {
        Self {
            ao: 1.0,
            colors: HashMap::new(),
            parts: HashMap::new(),
            part: None,
        }
    }
}

impl VoxelLook {
    /// Parts in order of first use, and the plan that maps palette values onto them.
    fn plan(&self, palette: &VoxelPalette) -> (Vec<VoxelKey>, MeshPlan) {
        let mut part_keys: Vec<VoxelKey> = Vec::new();
        let mut plan = MeshPlan {
            part_count: 0,
            part_of: [0; 256],
            colors: [[1.0; 4]; 256],
            ao: self.ao > 0.0,
            shade: AO_LEVELS.map(|level| 1.0 - self.ao.clamp(0.0, 1.0) * (1.0 - level)),
        };
        for (i, key) in palette.keys().iter().enumerate() {
            let part = self.parts.get(key).or(self.part.as_ref()).unwrap_or(key);
            let index = match part_keys.iter().position(|k| k == part) {
                Some(index) => index,
                None => {
                    part_keys.push(part.clone());
                    part_keys.len() - 1
                }
            };
            plan.part_of[i + 1] = index;
            if let Some(color) = self.colors.get(key) {
                plan.colors[i + 1] = *color;
            }
        }
        plan.part_count = part_keys.len();
        (part_keys, plan)
    }
}

/// One merged rectangle in slice `plane` of axis `d`, spanning `[i, i+w] x [j, j+h]` along
/// the two other axes (`d+1`, `d+2` in cyclic order).
struct Face {
    d: usize,
    positive: bool,
    plane: usize,
    i: usize,
    j: usize,
    w: usize,
    h: usize,
}

#[derive(Default)]
struct PartBuilder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    quads: usize,
}

impl PartBuilder {
    /// `shade` is the brightness of each corner, in quad order.
    fn quad(
        &mut self,
        face: Face,
        voxel_size: Vec3,
        origin: Vec3,
        color: [f32; 4],
        shade: [f32; 4],
    ) {
        let Face {
            d,
            positive,
            plane,
            i,
            j,
            w,
            h,
        } = face;
        let (a, b) = ((d + 1) % 3, (d + 2) % 3);
        let corner = |da: usize, db: usize| {
            let mut p = [0.0f32; 3];
            p[d] = plane as f32;
            p[a] = (i + da) as f32;
            p[b] = (j + db) as f32;
            Vec3::from(p)
        };
        // e_a x e_b = e_d, so this order winds counter-clockwise seen from +d.
        let mut corners = [corner(0, 0), corner(w, 0), corner(w, h), corner(0, h)];
        let mut shade = shade;
        if !positive {
            corners.reverse();
            shade.reverse();
        }

        let sign = if positive { 1.0 } else { -1.0 };
        let mut normal = Vec3::ZERO;
        normal[d] = sign;
        // Tangent along U, bitangent = N x T along V, as the primitive cube lays them out:
        // V runs down the side faces.
        let tangent = match (d, positive) {
            (0, true) => Vec3::Z,
            (0, false) => Vec3::NEG_Z,
            (1, _) => Vec3::X,
            (2, true) => Vec3::NEG_X,
            _ => Vec3::X,
        };
        let bitangent = normal.cross(tangent);

        let base = self.vertices.len() as u32;
        for (grid_pos, light) in corners.into_iter().zip(shade) {
            self.vertices.push(Vertex {
                pos: (origin + grid_pos * voxel_size).to_array(),
                normal: normal.to_array(),
                // In voxels, so a texture repeats once per voxel.
                uv: [grid_pos.dot(tangent), grid_pos.dot(bitangent)],
                tangent: [tangent.x, tangent.y, tangent.z, 1.0],
                color: [
                    color[0] * light,
                    color[1] * light,
                    color[2] * light,
                    color[3],
                ],
            });
        }
        // Split along the diagonal whose corners are darker, so a dark corner fades
        // evenly instead of streaking along the other diagonal.
        if shade[0] + shade[2] > shade[1] + shade[3] {
            self.indices.extend_from_slice(&[
                base,
                base + 1,
                base + 3,
                base + 1,
                base + 2,
                base + 3,
            ]);
        } else {
            self.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        self.quads += 1;
    }
}

/// The palette entries of a model in the order they were first used; the grid stores
/// `position + 1`.
#[derive(Debug, Default, Clone)]
pub struct VoxelPalette {
    keys: Vec<VoxelKey>,
    lookup: HashMap<VoxelKey, u8>,
}

impl VoxelPalette {
    /// The grid value for `key`, adding it if new.
    pub fn intern(&mut self, key: VoxelKey) -> Result<u8, String> {
        if let Some(&value) = self.lookup.get(&key) {
            return Ok(value);
        }
        if self.keys.len() >= MAX_KEYS {
            return Err(format!(
                "a voxel model can use at most {MAX_KEYS} different keys"
            ));
        }
        self.keys.push(key.clone());
        let value = self.keys.len() as u8;
        self.lookup.insert(key, value);
        Ok(value)
    }

    pub fn keys(&self) -> &[VoxelKey] {
        &self.keys
    }
}

/// Where a model's cells come from. Text art (`layers`) is read like a sprite: each layer
/// is rows from top to bottom, and the first layer is the front (+Z).
#[derive(Debug, Clone)]
pub struct VoxelSpec {
    /// Text layers, front to back; each a list of rows, top to bottom.
    pub layers: Option<Vec<Vec<String>>>,
    /// Text-art character -> key, or `None` for empty. Without a legend every character
    /// except `.` and space is its own key.
    pub legend: Option<HashMap<char, Option<VoxelKey>>>,
    /// How many voxels thick each text layer is.
    pub depth: u32,
    /// Grid size for `voxels` and fill functions.
    pub size: Option<[u32; 3]>,
    /// Flat cells, `x + W * (y + H * z)`, `None` for empty.
    pub voxels: Option<Vec<Option<VoxelKey>>>,
    /// Size of one voxel in meters.
    pub voxel_size: Vec3,
    /// Fraction of the model's box at the local origin: (0.5, 0, 0.5) stands the model on
    /// the origin, (0.5, 0.5, 0.5) centres it.
    pub pivot: Vec3,
    /// Shading and mesh grouping.
    pub look: VoxelLook,
}

impl Default for VoxelSpec {
    fn default() -> Self {
        Self {
            layers: None,
            legend: None,
            depth: 1,
            size: None,
            voxels: None,
            voxel_size: Vec3::splat(0.1),
            pivot: Vec3::new(0.5, 0.0, 0.5),
            look: VoxelLook::default(),
        }
    }
}

/// Called for every cell `(x, y, z)` of a `size`d grid; returns its key or `None`.
pub type FillFn<'a> = dyn FnMut(u32, u32, u32) -> Result<Option<VoxelKey>, String> + 'a;

impl VoxelSpec {
    /// Build the grid from the text layers, the flat `voxels` or `fill` (exactly one).
    pub fn grid(&self, fill: Option<&mut FillFn>) -> Result<(VoxelGrid, VoxelPalette), String> {
        let sources = [self.layers.is_some(), self.voxels.is_some(), fill.is_some()];
        match sources.iter().filter(|&&given| given).count() {
            1 => {}
            0 => return Err("a voxel model needs rows, layers, voxels or fill".into()),
            _ => return Err("give only one of rows/layers, voxels and fill".into()),
        }
        let mut palette = VoxelPalette::default();
        if let Some(layers) = &self.layers {
            let grid = self.text_grid(layers, &mut palette)?;
            return Ok((grid, palette));
        }
        let size = self
            .size
            .ok_or("voxels and fill need size = {width, height, depth}")?;
        let mut grid = VoxelGrid::new(size)?;
        if let Some(voxels) = &self.voxels {
            if voxels.len() != grid.cells.len() {
                return Err(format!(
                    "voxels has {} entries but size {size:?} needs {}",
                    voxels.len(),
                    grid.cells.len()
                ));
            }
            for (cell, key) in grid.cells.iter_mut().zip(voxels) {
                if let Some(key) = key {
                    *cell = palette.intern(key.clone())?;
                }
            }
        } else if let Some(fill) = fill {
            for z in 0..size[2] {
                for y in 0..size[1] {
                    for x in 0..size[0] {
                        if let Some(key) = fill(x, y, z)? {
                            let value = palette.intern(key)?;
                            grid.set(x, y, z, value);
                        }
                    }
                }
            }
        }
        Ok((grid, palette))
    }

    fn text_grid(
        &self,
        layers: &[Vec<String>],
        palette: &mut VoxelPalette,
    ) -> Result<VoxelGrid, String> {
        let depth = self.depth.max(1);
        let height = layers.first().map_or(0, Vec::len);
        if layers.is_empty() || height == 0 {
            return Err("rows/layers is empty".into());
        }
        let width = layers[0][0].chars().count();
        for (l, rows) in layers.iter().enumerate() {
            if rows.len() != height {
                return Err(format!(
                    "layer {} has {} rows, expected {height}",
                    l + 1,
                    rows.len()
                ));
            }
            for (r, row) in rows.iter().enumerate() {
                let len = row.chars().count();
                if len != width {
                    return Err(format!(
                        "layer {} row {} is {len} wide, expected {width}",
                        l + 1,
                        r + 1
                    ));
                }
            }
        }
        let total_depth = layers.len() as u64 * depth as u64;
        if width == 0 || width > MAX_DIMENSION as usize || total_depth > MAX_DIMENSION as u64 {
            return Err(format!(
                "voxel model is {width} x {height} x {total_depth}; each side must be 1..{MAX_DIMENSION}"
            ));
        }
        let size = [width as u32, height as u32, total_depth as u32];
        let mut grid = VoxelGrid::new(size)?;

        let mut values: HashMap<char, u8> = HashMap::new();
        for (l, rows) in layers.iter().enumerate() {
            for (r, row) in rows.iter().enumerate() {
                let y = (height - 1 - r) as u32;
                for (x, ch) in row.chars().enumerate() {
                    let value = match values.get(&ch) {
                        Some(&value) => value,
                        None => {
                            let key = match &self.legend {
                                _ if ch == '.' || ch == ' ' => None,
                                Some(legend) => legend
                                    .get(&ch)
                                    .ok_or_else(|| format!("the legend has no entry for {ch:?}"))?
                                    .clone(),
                                None => Some(VoxelKey::Str(ch.to_string())),
                            };
                            let value = match key {
                                Some(key) => palette.intern(key)?,
                                None => 0,
                            };
                            values.insert(ch, value);
                            value
                        }
                    };
                    if value == 0 {
                        continue;
                    }
                    // Layer 1 is the front, so it gets the highest z.
                    let back = size[2] - (l as u32 + 1) * depth;
                    for z in back..back + depth {
                        grid.set(x as u32, y, z, value);
                    }
                }
            }
        }
        Ok(grid)
    }
}

/// One mesh of a model: the cells of one key (or of the keys [`VoxelLook::parts`] groups
/// under one part key), meshed.
#[derive(Debug, Clone)]
pub struct VoxelPart {
    pub key: VoxelKey,
    /// The palette keys drawn by this part.
    pub keys: Vec<VoxelKey>,
    pub mesh: Arc<GeneratedMesh>,
    pub quads: usize,
    pub voxels: usize,
}

/// A meshed voxel model.
#[derive(Debug, Clone)]
pub struct VoxelModel {
    pub size: [u32; 3],
    pub voxel_size: Vec3,
    /// Local-space position of the grid corner (0, 0, 0).
    pub origin: Vec3,
    pub parts: Vec<VoxelPart>,
    pub voxels: usize,
}

impl VoxelModel {
    /// Mesh `grid` with the size, pivot and look of `spec`.
    pub fn build(grid: &VoxelGrid, palette: &VoxelPalette, spec: &VoxelSpec) -> Self {
        let (voxel_size, pivot) = (spec.voxel_size, spec.pivot);
        let size = grid.size();
        let origin =
            -(Vec3::new(size[0] as f32, size[1] as f32, size[2] as f32) * pivot) * voxel_size;
        let (part_keys, plan) = spec.look.plan(palette);

        let mut hasher = DefaultHasher::new();
        grid.hash(&mut hasher);
        voxel_size.to_array().map(f32::to_bits).hash(&mut hasher);
        origin.to_array().map(f32::to_bits).hash(&mut hasher);
        plan.part_of.hash(&mut hasher);
        plan.colors
            .iter()
            .for_each(|color| color.map(f32::to_bits).hash(&mut hasher));
        plan.ao.hash(&mut hasher);
        plan.shade.map(f32::to_bits).hash(&mut hasher);
        let grid_key = hasher.finish();

        let mut voxel_counts = [0usize; 256];
        for &cell in &grid.cells {
            voxel_counts[cell as usize] += 1;
        }

        let parts = grid
            .mesh_parts(voxel_size, origin, &plan)
            .into_iter()
            .zip(part_keys)
            .enumerate()
            .filter_map(|(index, (part, key))| {
                let values: Vec<usize> = (1..=palette.keys().len())
                    .filter(|&value| plan.part_of[value] == index && voxel_counts[value] > 0)
                    .collect();
                if values.is_empty() {
                    return None;
                }
                let mut hasher = DefaultHasher::new();
                grid_key.hash(&mut hasher);
                index.hash(&mut hasher);
                Some(VoxelPart {
                    key,
                    keys: values
                        .iter()
                        .map(|&value| palette.keys()[value - 1].clone())
                        .collect(),
                    mesh: GeneratedMesh::new(
                        hasher.finish(),
                        MeshData::new(part.vertices, part.indices),
                    ),
                    quads: part.quads,
                    voxels: values.iter().map(|&value| voxel_counts[value]).sum(),
                })
            })
            .collect();

        Self {
            size,
            voxel_size,
            origin,
            parts,
            voxels: grid.solid_count(),
        }
    }

    pub fn quads(&self) -> usize {
        self.parts.iter().map(|part| part.quads).sum()
    }

    pub fn part(&self, key: &VoxelKey) -> Option<&VoxelPart> {
        self.parts.iter().find(|part| &part.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(layers: &[&[&str]], depth: u32) -> VoxelModel {
        let spec = VoxelSpec {
            layers: Some(
                layers
                    .iter()
                    .map(|rows| rows.iter().map(|row| row.to_string()).collect())
                    .collect(),
            ),
            depth,
            voxel_size: Vec3::ONE,
            pivot: Vec3::ZERO,
            ..VoxelSpec::default()
        };
        let (grid, palette) = spec.grid(None).unwrap();
        VoxelModel::build(&grid, &palette, &spec)
    }

    fn key(s: &str) -> VoxelKey {
        VoxelKey::Str(s.into())
    }

    #[test]
    fn a_solid_box_is_six_quads() {
        let model = text(&[&["AAAA", "AAAA", "AAAA"]], 5);
        assert_eq!(model.size, [4, 3, 5]);
        assert_eq!(model.voxels, 60);
        assert_eq!(model.parts.len(), 1);
        assert_eq!(model.quads(), 6);
        let mesh = &model.parts[0].mesh;
        assert_eq!(mesh.data.vertices.len(), 24);
        assert_eq!(mesh.triangles(), 12);
        assert_eq!(mesh.min, Vec3::ZERO);
        assert_eq!(mesh.max, Vec3::new(4.0, 3.0, 5.0));
    }

    #[test]
    fn faces_between_colours_are_hidden() {
        // Two cubes side by side: 10 outer faces, the shared one is not drawn.
        let model = text(&[&["AB"]], 1);
        assert_eq!(model.parts.len(), 2);
        assert_eq!(model.part(&key("A")).unwrap().quads, 5);
        assert_eq!(model.part(&key("B")).unwrap().quads, 5);
    }

    #[test]
    fn faces_point_outward_and_wind_counter_clockwise() {
        let model = text(&[&["A.A", "AAA"]], 2);
        let mesh = &model.parts[0].mesh;
        for tri in mesh.data.indices.chunks(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| mesh.data.vertices[i as usize]);
            let (pa, pb, pc) = (Vec3::from(a.pos), Vec3::from(b.pos), Vec3::from(c.pos));
            let geometric = (pb - pa).cross(pc - pa).normalize();
            let normal = Vec3::from(a.normal);
            assert!(
                geometric.dot(normal) > 0.99,
                "winding disagrees with the normal"
            );
            // The tangent lies in the face and the stored bitangent sign is consistent.
            assert!(Vec3::from_slice(&a.tangent[..3]).dot(normal).abs() < 1e-6);
            // A face never looks into a solid cell: its centre pushed along the normal is
            // outside the model or in the one empty cell.
            let probe = (pa + pb + pc) / 3.0 + normal * 0.25;
            let (x, y, z) = (probe.x.floor(), probe.y.floor(), probe.z.floor());
            let inside = x >= 0.0
                && y >= 0.0
                && z >= 0.0
                && (x as u32) < model.size[0]
                && (y as u32) < model.size[1]
                && (z as u32) < model.size[2];
            if inside {
                // The notch cell (x = 1, top row) is the only empty cell in the box.
                assert_eq!((x as u32, y as u32), (1, 1), "face looks into a solid cell");
            }
        }
    }

    #[test]
    fn greedy_merging_beats_one_cube_per_voxel() {
        // A 16x16x16 ball.
        let spec = VoxelSpec {
            size: Some([16, 16, 16]),
            voxel_size: Vec3::ONE,
            ..VoxelSpec::default()
        };
        let mut fill = |x: u32, y: u32, z: u32| {
            let p = Vec3::new(x as f32, y as f32, z as f32) - Vec3::splat(7.5);
            Ok((p.length() < 7.5).then_some(VoxelKey::Int(1)))
        };
        let (grid, palette) = spec.grid(Some(&mut fill)).unwrap();
        let model = VoxelModel::build(&grid, &palette, &spec);
        assert!(model.voxels > 1000);
        assert!(
            model.quads() * 10 < grid.naive_quads(),
            "{} quads for {} voxels",
            model.quads(),
            model.voxels
        );
    }

    #[test]
    fn the_surface_is_closed() {
        // Every edge of a closed surface is shared by exactly two triangles going opposite
        // ways. Greedy quads leave T-junctions, so check area instead: the surface area
        // of the mesh equals the number of exposed unit faces.
        let model = text(&[&["AABA", "A..A", "ABBA"], &["AAAA", "AAAA", "AAAA"]], 1);
        let area: f32 = model
            .parts
            .iter()
            .flat_map(|part| {
                let vertices = &part.mesh.data.vertices;
                part.mesh
                    .data
                    .indices
                    .chunks(3)
                    .map(move |tri| {
                        let [a, b, c] =
                            [tri[0], tri[1], tri[2]].map(|i| Vec3::from(vertices[i as usize].pos));
                        (b - a).cross(c - a).length() * 0.5
                    })
                    .collect::<Vec<_>>()
            })
            .sum();
        // Count exposed faces of the grid directly.
        let spec = VoxelSpec {
            layers: Some(vec![
                vec!["AABA".into(), "A..A".into(), "ABBA".into()],
                vec!["AAAA".into(), "AAAA".into(), "AAAA".into()],
            ]),
            ..VoxelSpec::default()
        };
        let (grid, _) = spec.grid(None).unwrap();
        let [w, h, d] = grid.size().map(|s| s as i32);
        let solid = |x: i32, y: i32, z: i32| {
            x >= 0
                && y >= 0
                && z >= 0
                && x < w
                && y < h
                && z < d
                && grid.get(x as u32, y as u32, z as u32) != 0
        };
        let mut exposed = 0;
        for z in 0..d {
            for y in 0..h {
                for x in 0..w {
                    if !solid(x, y, z) {
                        continue;
                    }
                    for (dx, dy, dz) in [
                        (1, 0, 0),
                        (-1, 0, 0),
                        (0, 1, 0),
                        (0, -1, 0),
                        (0, 0, 1),
                        (0, 0, -1),
                    ] {
                        if !solid(x + dx, y + dy, z + dz) {
                            exposed += 1;
                        }
                    }
                }
            }
        }
        assert!(
            (area - exposed as f32).abs() < 1e-3,
            "area {area}, exposed {exposed}"
        );
    }

    #[test]
    fn text_layers_run_top_down_and_front_to_back() {
        let spec = VoxelSpec {
            layers: Some(vec![
                vec!["A.".into(), "..".into()],
                vec!["..".into(), ".B".into()],
            ]),
            ..VoxelSpec::default()
        };
        let (grid, palette) = spec.grid(None).unwrap();
        assert_eq!(grid.size(), [2, 2, 2]);
        // A: first layer (front, z = 1), top row (y = 1), left column.
        assert_eq!(grid.get(0, 1, 1), 1);
        // B: second layer (back, z = 0), bottom row, right column.
        assert_eq!(grid.get(1, 0, 0), 2);
        assert_eq!(palette.keys(), &[key("A"), key("B")]);
    }

    #[test]
    fn legend_maps_characters_and_rejects_unknown_ones() {
        let mut legend = HashMap::new();
        legend.insert('x', Some(key("red")));
        legend.insert('o', None);
        let spec = VoxelSpec {
            layers: Some(vec![vec!["xox".into()]]),
            legend: Some(legend.clone()),
            ..VoxelSpec::default()
        };
        let (grid, palette) = spec.grid(None).unwrap();
        assert_eq!(grid.solid_count(), 2);
        assert_eq!(palette.keys(), &[key("red")]);

        let spec = VoxelSpec {
            layers: Some(vec![vec!["xq".into()]]),
            legend: Some(legend),
            ..VoxelSpec::default()
        };
        assert!(spec.grid(None).unwrap_err().contains("'q'"));
    }

    #[test]
    fn ragged_rows_and_wrong_voxel_counts_are_errors() {
        let spec = VoxelSpec {
            layers: Some(vec![vec!["AA".into(), "A".into()]]),
            ..VoxelSpec::default()
        };
        assert!(spec.grid(None).unwrap_err().contains("row 2"));
        let spec = VoxelSpec {
            size: Some([2, 2, 2]),
            voxels: Some(vec![None; 7]),
            ..VoxelSpec::default()
        };
        assert!(spec.grid(None).unwrap_err().contains("needs 8"));
    }

    #[test]
    fn pivot_places_the_model() {
        let spec = VoxelSpec {
            layers: Some(vec![vec!["AA".into(), "AA".into()]]),
            depth: 2,
            voxel_size: Vec3::splat(0.5),
            ..VoxelSpec::default()
        };
        let (grid, palette) = spec.grid(None).unwrap();
        let model = VoxelModel::build(&grid, &palette, &spec);
        let mesh = &model.parts[0].mesh;
        assert_eq!(mesh.min, Vec3::new(-0.5, 0.0, -0.5));
        assert_eq!(mesh.max, Vec3::new(0.5, 1.0, 0.5));
    }

    /// A 3x3 floor one voxel thick with a pillar of `B` on its centre.
    fn floor_and_pillar(look: VoxelLook) -> VoxelModel {
        let spec = VoxelSpec {
            size: Some([3, 2, 3]),
            voxel_size: Vec3::ONE,
            pivot: Vec3::ZERO,
            look,
            ..VoxelSpec::default()
        };
        let mut fill = |x: u32, y: u32, z: u32| {
            Ok(match (x, y, z) {
                (_, 0, _) => Some(key("A")),
                (1, 1, 1) => Some(key("B")),
                _ => None,
            })
        };
        let (grid, palette) = spec.grid(Some(&mut fill)).unwrap();
        VoxelModel::build(&grid, &palette, &spec)
    }

    fn top_face_colors(part: &VoxelPart, y: f32) -> Vec<(Vec3, [f32; 4])> {
        part.mesh
            .data
            .vertices
            .iter()
            .filter(|v| v.normal == [0.0, 1.0, 0.0] && v.pos[1] == y)
            .map(|v| (Vec3::from(v.pos), v.color))
            .collect()
    }

    #[test]
    fn ambient_occlusion_darkens_inner_corners_only() {
        let model = floor_and_pillar(VoxelLook::default());
        let floor = model.part(&key("A")).unwrap();
        let top = top_face_colors(floor, 1.0);
        assert!(!top.is_empty());
        for (pos, color) in &top {
            // The pillar stands on [1, 2] x [1, 2]; corners touching it are shaded.
            let touches = (1.0..=2.0).contains(&pos.x) && (1.0..=2.0).contains(&pos.z);
            if touches {
                assert!(
                    color[0] < 0.99,
                    "corner {pos} next to the pillar is not shaded"
                );
            } else {
                assert_eq!(color[0], 1.0, "open corner {pos} is shaded");
            }
            assert_eq!(color[3], 1.0);
        }
        // Nothing stands on the pillar, so its top is unshaded and one quad.
        let pillar = model.part(&key("B")).unwrap();
        let top = top_face_colors(pillar, 2.0);
        assert_eq!(top.len(), 4);
        assert!(top.iter().all(|(_, color)| color[0] == 1.0));
        // The floor's top cannot be one quad any more: it is split where shading changes.
        assert!(floor.quads > 6);
    }

    #[test]
    fn ambient_occlusion_can_be_turned_off() {
        let model = floor_and_pillar(VoxelLook {
            ao: 0.0,
            ..VoxelLook::default()
        });
        for part in &model.parts {
            assert!(part
                .mesh
                .data
                .vertices
                .iter()
                .all(|v| v.color == Vertex::WHITE));
        }
        // Without shading the floor's top is a ring around the pillar's footprint;
        // with it, more quads.
        let shaded = floor_and_pillar(VoxelLook::default());
        assert!(model.quads() < shaded.quads());
    }

    #[test]
    fn grouped_keys_share_a_mesh_with_their_own_colours() {
        let mut look = VoxelLook {
            ao: 0.0,
            ..VoxelLook::default()
        };
        look.parts.insert(key("A"), key("body"));
        look.parts.insert(key("B"), key("body"));
        look.colors.insert(key("A"), [1.0, 0.0, 0.0, 1.0]);
        look.colors.insert(key("B"), [0.0, 0.0, 1.0, 1.0]);
        let spec = VoxelSpec {
            layers: Some(vec![vec!["AB".into(), "C.".into()]]),
            look,
            ..VoxelSpec::default()
        };
        let (grid, palette) = spec.grid(None).unwrap();
        let model = VoxelModel::build(&grid, &palette, &spec);
        assert_eq!(model.parts.len(), 2);
        let body = model.part(&key("body")).unwrap();
        assert_eq!(body.keys, vec![key("A"), key("B")]);
        assert_eq!(body.voxels, 2);
        let colors: Vec<[f32; 4]> = body.mesh.data.vertices.iter().map(|v| v.color).collect();
        assert!(colors.contains(&[1.0, 0.0, 0.0, 1.0]));
        assert!(colors.contains(&[0.0, 0.0, 1.0, 1.0]));
        // Unlisted keys keep their own white part.
        let c = model.part(&key("C")).unwrap();
        assert!(c
            .mesh
            .data
            .vertices
            .iter()
            .all(|v| v.color == Vertex::WHITE));
    }

    #[test]
    fn shading_does_not_change_the_surface() {
        // Same area with and without AO: shading only splits quads.
        let area = |model: &VoxelModel| -> f32 {
            model
                .parts
                .iter()
                .flat_map(|part| {
                    let v = &part.mesh.data.vertices;
                    part.mesh.data.indices.chunks(3).map(move |t| {
                        let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(v[i as usize].pos));
                        (b - a).cross(c - a).length() * 0.5
                    })
                })
                .sum()
        };
        let lit = floor_and_pillar(VoxelLook::default());
        let flat = floor_and_pillar(VoxelLook {
            ao: 0.0,
            ..VoxelLook::default()
        });
        // 3x3x1 floor (18 + 12 sides = 30 faces) + pillar (5 faces) - 1 hidden = 34.
        assert!((area(&lit) - 34.0).abs() < 1e-4, "{}", area(&lit));
        assert!((area(&flat) - 34.0).abs() < 1e-4);
    }

    #[test]
    fn same_content_gives_same_keys() {
        let a = text(&[&["AB"]], 1);
        let b = text(&[&["AB"]], 1);
        let c = text(&[&["A.B"]], 1);
        assert_eq!(a.parts[0].mesh.key, b.parts[0].mesh.key);
        assert_ne!(a.parts[0].mesh.key, a.parts[1].mesh.key);
        assert_ne!(a.parts[0].mesh.key, c.parts[0].mesh.key);
    }
}
