//! The baked navmesh file (`content/navigation/*.navmesh`): the tiles' polygons and
//! height detail, little-endian. Links are not stored; they are rebuilt as the tiles
//! are added.

use glam::Vec3;

use super::builder::TileGrid;
use super::navmesh::{NavMesh, NavPoly, NavTile, PolyKind, MAX_VERTS_PER_POLY};

const MAGIC: &[u8; 4] = b"WCNM";
const VERSION: u32 = 1;

/// What a navmesh file holds besides the tiles.
#[derive(Debug, Clone, PartialEq)]
pub struct NavMeshFileInfo {
    /// [`super::NavMeshSurface::build_settings_hash`] of the settings it was baked with.
    pub settings_hash: u64,
    pub grid: TileGrid,
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).ok_or("navmesh file is corrupt")?;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or("navmesh file is truncated")?;
        self.at = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().map_err(|_| "bad u16")?,
        ))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| "bad u32")?,
        ))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| "bad i32")?,
        ))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| "bad u64")?,
        ))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| "bad f32")?,
        ))
    }
    fn vec3(&mut self) -> Result<Vec3, String> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    /// A count, checked against what is left of the file (`unit` bytes per item).
    fn count(&mut self, unit: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n.saturating_mul(unit) > self.bytes.len() - self.at {
            return Err("navmesh file is corrupt".into());
        }
        Ok(n)
    }
}

/// Encode a navmesh (ground polygons only; off-mesh links come from the scene).
pub fn encode(mesh: &NavMesh, info: &NavMeshFileInfo) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.0.extend_from_slice(MAGIC);
    w.u32(VERSION);
    w.u64(info.settings_hash);
    w.vec3(info.grid.origin);
    w.f32(info.grid.max_y);
    w.i32(info.grid.tiles_x);
    w.i32(info.grid.tiles_z);
    w.f32(info.grid.tile_size);
    w.f32(mesh.walkable_climb);
    let tiles: Vec<&NavTile> = mesh.tiles().collect();
    w.u32(tiles.len() as u32);
    for tile in tiles {
        w.i32(tile.x);
        w.i32(tile.z);
        w.vec3(tile.bmin);
        w.vec3(tile.bmax);
        let ground = tile.ground_count;
        // Vertices used by ground polygons come first (off-mesh ones are appended).
        let vert_count = tile.polys[..ground]
            .iter()
            .flat_map(|p| p.verts[..usize::from(p.vert_count)].iter())
            .map(|&v| usize::from(v) + 1)
            .max()
            .unwrap_or(0);
        w.u32(vert_count as u32);
        for v in &tile.verts[..vert_count] {
            w.vec3(*v);
        }
        w.u32(ground as u32);
        for poly in &tile.polys[..ground] {
            w.u8(poly.vert_count);
            w.u8(poly.area);
            for k in 0..MAX_VERTS_PER_POLY {
                w.u16(poly.verts[k]);
                w.u16(poly.neis[k]);
            }
        }
        for m in &tile.detail_meshes[..ground.min(tile.detail_meshes.len())] {
            for v in m {
                w.u32(*v);
            }
        }
        w.u32(tile.detail_verts.len() as u32);
        for v in &tile.detail_verts {
            w.vec3(*v);
        }
        w.u32(tile.detail_tris.len() as u32);
        for t in &tile.detail_tris {
            w.u8(t[0]);
            w.u8(t[1]);
            w.u8(t[2]);
        }
    }
    w.0
}

/// Decode a navmesh file.
pub fn decode(bytes: &[u8]) -> Result<(NavMesh, NavMeshFileInfo), String> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(4)? != MAGIC {
        return Err("not a navmesh file".into());
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(format!(
            "navmesh file version {version} is not supported (expected {VERSION})"
        ));
    }
    let settings_hash = r.u64()?;
    let origin = r.vec3()?;
    let max_y = r.f32()?;
    let tiles_x = r.i32()?;
    let tiles_z = r.i32()?;
    let tile_size = r.f32()?;
    let walkable_climb = r.f32()?;
    if tile_size.is_nan() || tile_size <= 0.0 {
        return Err("navmesh file has no tile size".into());
    }
    let mut mesh = NavMesh::new(origin, tile_size, walkable_climb);
    let tile_count = r.count(32)?;
    for _ in 0..tile_count {
        let x = r.i32()?;
        let z = r.i32()?;
        let bmin = r.vec3()?;
        let bmax = r.vec3()?;
        let nverts = r.count(12)?;
        let verts = (0..nverts)
            .map(|_| r.vec3())
            .collect::<Result<Vec<_>, _>>()?;
        let npolys = r.count(2 + MAX_VERTS_PER_POLY * 4)?;
        let mut polys = Vec::with_capacity(npolys);
        for _ in 0..npolys {
            let vert_count = r.u8()?;
            let area = r.u8()?;
            let mut poly = NavPoly {
                verts: [0; MAX_VERTS_PER_POLY],
                vert_count: vert_count.min(MAX_VERTS_PER_POLY as u8),
                neis: [0; MAX_VERTS_PER_POLY],
                area,
                kind: PolyKind::Ground,
            };
            for k in 0..MAX_VERTS_PER_POLY {
                poly.verts[k] = r.u16()?;
                poly.neis[k] = r.u16()?;
            }
            if poly.verts[..usize::from(poly.vert_count)]
                .iter()
                .any(|&v| usize::from(v) >= nverts)
            {
                return Err("navmesh file has a polygon outside its vertices".into());
            }
            polys.push(poly);
        }
        let mut detail_meshes = Vec::with_capacity(npolys);
        for _ in 0..npolys {
            detail_meshes.push([r.u32()?, r.u32()?, r.u32()?, r.u32()?]);
        }
        let ndverts = r.count(12)?;
        let detail_verts = (0..ndverts)
            .map(|_| r.vec3())
            .collect::<Result<Vec<_>, _>>()?;
        let ndtris = r.count(3)?;
        let mut detail_tris = Vec::with_capacity(ndtris);
        for _ in 0..ndtris {
            detail_tris.push([r.u8()?, r.u8()?, r.u8()?]);
        }
        for m in &detail_meshes {
            let verts_ok = (m[0] as usize).saturating_add(m[1] as usize) <= ndverts;
            let tris_ok = (m[2] as usize).saturating_add(m[3] as usize) <= ndtris;
            if !verts_ok || !tris_ok {
                return Err("navmesh file has detail outside its arrays".into());
            }
        }
        let mut tile = NavTile {
            x,
            z,
            salt: 0,
            bmin,
            bmax,
            verts,
            links: vec![Vec::new(); polys.len()],
            ground_count: polys.len(),
            polys,
            detail_meshes,
            detail_verts,
            detail_tris,
            poly_bounds: Vec::new(),
            off_mesh: Vec::new(),
        };
        tile.compute_bounds();
        mesh.add_tile(tile);
    }
    Ok((
        mesh,
        NavMeshFileInfo {
            settings_hash,
            grid: TileGrid {
                origin,
                max_y,
                tiles_x,
                tiles_z,
                tile_size,
            },
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"nope").is_err());
        assert!(decode(b"WCNM\x63\0\0\0").is_err());
    }
}
