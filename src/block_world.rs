use super::*;

/// Blocks along one edge of a streamed chunk. This is a Minecraft chunk section:
/// 16×16×16 blocks.
pub const BLOCKS_PER_CHUNK: i32 = 16;
/// Chunks along one edge of the cube the raytracer can see at once. 2^3 = 8.
/// The cube slides as you fly, so chunk coordinates themselves are unbounded.
pub const WINDOW_CHUNKS: i32 = 8;
pub const WINDOW_DEPTH: u32 = 3;
/// Octree depth inside one section. 2^4 = 16.
pub const CHUNK_DEPTH: u32 = 4;

pub const STONE: u32 = 1;
pub const DIRT: u32 = 2;
pub const GRASS: u32 = 3;
pub const WOOD: u32 = 4;
pub const LEAF: u32 = 5;

/// Terrain chunk ids live at and above this value so unsubdivision can drop them
/// without touching the resident block models (ids 1–8).
pub const STREAM_BASE: u32 = CHUNK_OFFSET / 2;

/// Center of cell `i` in a grid of `count` cells spanning the octree's [-1, 1] cube.
pub fn cell_center(i: i32, count: i32) -> f32 {
    -1.0 + (i as f32 + 0.5) / count as f32 * 2.0
}

pub fn window_span() -> i32 {
    WINDOW_CHUNKS * BLOCKS_PER_CHUNK
}

/// Octree position of a world block, relative to the window whose minimum chunk is `origin`.
pub fn block_window_pos(wx: i32, wy: i32, wz: i32, origin: (i32, i32, i32)) -> Vector3<f32> {
    let span = window_span();
    Vector3::new(
        cell_center(wx - origin.0 * BLOCKS_PER_CHUNK, span),
        cell_center(wy - origin.1 * BLOCKS_PER_CHUNK, span),
        cell_center(wz - origin.2 * BLOCKS_PER_CHUNK, span),
    )
}

pub fn blocks_to_octree(block: Vector3<f32>, origin: (i32, i32, i32)) -> Vector3<f32> {
    let span = window_span() as f32;
    let origin_block = Vector3::new(
        (origin.0 * BLOCKS_PER_CHUNK) as f32,
        (origin.1 * BLOCKS_PER_CHUNK) as f32,
        (origin.2 * BLOCKS_PER_CHUNK) as f32,
    );
    (block - origin_block) / span * 2.0 - Vector3::new(1.0, 1.0, 1.0)
}

pub fn octree_to_blocks(octree: Vector3<f32>, origin: (i32, i32, i32)) -> Vector3<f32> {
    let span = window_span() as f32;
    let origin_block = Vector3::new(
        (origin.0 * BLOCKS_PER_CHUNK) as f32,
        (origin.1 * BLOCKS_PER_CHUNK) as f32,
        (origin.2 * BLOCKS_PER_CHUNK) as f32,
    );
    (octree + Vector3::new(1.0, 1.0, 1.0)) * 0.5 * span + origin_block
}

/// Center of a chunk at a local index inside the traced window.
pub fn chunk_center(local_x: i32, local_y: i32, local_z: i32) -> Vector3<f32> {
    Vector3::new(
        cell_center(local_x, WINDOW_CHUNKS),
        cell_center(local_y, WINDOW_CHUNKS),
        cell_center(local_z, WINDOW_CHUNKS),
    )
}

pub fn chunk_of_block(b: i32) -> i32 {
    b.div_euclid(BLOCKS_PER_CHUNK)
}

pub fn window_origin_for_chunk(cx: i32, cy: i32, cz: i32) -> (i32, i32, i32) {
    (
        cx - WINDOW_CHUNKS / 2,
        cy - WINDOW_CHUNKS / 2,
        cz - WINDOW_CHUNKS / 2,
    )
}

/// Continuous height. The block world uses this rounded to an integer.
/// The far-field march in `shader.wgsl` duplicates this and `terrain_gradient`.
pub fn terrain_surface(x: f32, z: f32) -> f32 {
    72.0
        + (x * 0.05).sin() * 6.0
        + (z * 0.05).cos() * 6.0
        + ((x + z) * 0.02).sin() * 14.0
        + (x * 0.01 - z * 0.013).cos() * 10.0
}

/// (dh/dx, dh/dz). Each component is bounded, and the euclidean slope stays under 1.05.
pub fn terrain_gradient(x: f32, z: f32) -> (f32, f32) {
    let a = x * 0.05;
    let b = z * 0.05;
    let c = (x + z) * 0.02;
    let d = x * 0.01 - z * 0.013;
    let dhdx = a.cos() * 0.30 + c.cos() * 0.28 - d.sin() * 0.10;
    let dhdz = -b.sin() * 0.30 + c.cos() * 0.28 + d.sin() * 0.13;
    (dhdx, dhdz)
}

pub const TERRAIN_MAX_SLOPE: f32 = 1.05;
/// Highest the sines can reach: 72 + 6 + 6 + 14 + 10. Air above this is empty.
pub const TERRAIN_MAX_HEIGHT: f32 = 108.0;

pub fn terrain_height(x: i32, z: i32) -> i32 {
    terrain_surface(x as f32, z as f32).round() as i32
}

/// Walk `dir` from `origin` in block space until it meets the height sheet.
///
/// Above the crests the ray jumps to `TERRAIN_MAX_HEIGHT`. Under that, each step
/// is `gap / approach`, with `approach` from `TERRAIN_MAX_SLOPE`. That bound sits
/// above the real gradient, so the step meets the sheet instead of skipping a hill.
pub fn march_terrain(origin: Vector3<f32>, dir: Vector3<f32>) -> Option<Vector3<f32>> {
    let dir = dir.normalize();
    let horiz = (dir.x * dir.x + dir.z * dir.z).sqrt();
    let approach = TERRAIN_MAX_SLOPE * horiz - dir.y;
    let mut p = origin;
    if approach <= 0.02 {
        let h = terrain_surface(p.x, p.z);
        if p.y <= h {
            return Some(Vector3::new(p.x, h, p.z));
        }
        return None;
    }

    if p.y > TERRAIN_MAX_HEIGHT && dir.y < 0.0 {
        p += dir * ((p.y - TERRAIN_MAX_HEIGHT) / -dir.y);
    }

    for _ in 0..96 {
        let h = terrain_surface(p.x, p.z);
        let g = p.y - h;
        if g <= 0.05 {
            return Some(Vector3::new(p.x, h, p.z));
        }
        p += dir * (g / approach);
    }
    None
}

fn hash(x: i32, y: i32, z: i32) -> u32 {
    let mut n = x
        .wrapping_mul(374761393)
        .wrapping_add(y.wrapping_mul(668265263))
        .wrapping_add(z.wrapping_mul(1274126177));
    n = (n ^ (n >> 13)).wrapping_mul(1274126177);
    (n ^ (n >> 16)) as u32
}

fn is_tree(x: i32, z: i32) -> bool {
    hash(x, 1, z) % 56 == 0
}

/// Block id at world block coordinates. 0 is air.
pub fn block_at(wx: i32, wy: i32, wz: i32) -> u32 {
    let h = terrain_height(wx, wz);
    if wy < h - 3 {
        return STONE;
    }
    if wy < h {
        return DIRT;
    }
    if wy == h {
        return GRASS;
    }

    for ax in wx - 2..=wx + 2 {
        for az in wz - 2..=wz + 2 {
            if !is_tree(ax, az) {
                continue;
            }
            let th = terrain_height(ax, az);
            let dx = wx - ax;
            let dz = wz - az;
            if dx == 0 && dz == 0 && wy > th && wy <= th + 4 {
                return WOOD;
            }
            if wy >= th + 3 && wy <= th + 5 && dx * dx + dz * dz <= 4 && wy > th {
                return LEAF;
            }
        }
    }

    0
}

/// Representative block id if this section contains anything, used as the far color.
pub fn chunk_kind(cx: i32, cy: i32, cz: i32) -> Option<u32> {
    let x0 = cx * BLOCKS_PER_CHUNK;
    let z0 = cz * BLOCKS_PER_CHUNK;
    let y0 = cy * BLOCKS_PER_CHUNK;
    let y1 = y0 + BLOCKS_PER_CHUNK - 1;

    let mut min_h = i32::MAX;
    let mut max_h = i32::MIN;
    let mut foliage = false;

    for lx in -2..BLOCKS_PER_CHUNK + 2 {
        for lz in -2..BLOCKS_PER_CHUNK + 2 {
            let wx = x0 + lx;
            let wz = z0 + lz;
            let h = terrain_height(wx, wz);
            let inside = (0..BLOCKS_PER_CHUNK).contains(&lx) && (0..BLOCKS_PER_CHUNK).contains(&lz);
            if inside {
                min_h = min_h.min(h);
                max_h = max_h.max(h);
            }
            if is_tree(wx, wz) && y1 > h && y0 <= h + 6 {
                let nearest_x = wx.clamp(x0, x0 + BLOCKS_PER_CHUNK - 1);
                let nearest_z = wz.clamp(z0, z0 + BLOCKS_PER_CHUNK - 1);
                let dx = wx - nearest_x;
                let dz = wz - nearest_z;
                if dx * dx + dz * dz <= 4 {
                    foliage = true;
                }
            }
        }
    }

    if min_h == i32::MAX || (y0 > max_h + 6 && !foliage) {
        return None;
    }
    if y0 > max_h {
        Some(LEAF)
    } else if y1 < min_h - 3 {
        Some(STONE)
    } else if y1 < min_h {
        Some(DIRT)
    } else {
        Some(GRASS)
    }
}

fn block_index(x: i32, y: i32, z: i32) -> usize {
    ((x as usize) << 8) | ((y as usize) << 4) | z as usize
}

fn region_empty(ids: &[u32], ox: i32, oy: i32, oz: i32, size: i32) -> bool {
    for y in oy..oy + size {
        for x in ox..ox + size {
            for z in oz..oz + size {
                if ids[block_index(x, y, z)] != 0 {
                    return false;
                }
            }
        }
    }
    true
}

/// `size` is the blocks on an edge of this group. The 8 children are written at the
/// returned index. A size of 2 means those children are individual blocks.
fn build_group(nodes: &mut Vec<Node>, ids: &[u32], ox: i32, oy: i32, oz: i32, size: i32) -> usize {
    let index = nodes.len();
    let air = Node::new(CHUNK_OFFSET, Voxel::new(0, 0, 0));
    for _ in 0..8 {
        nodes.push(air);
    }

    let h = size / 2;
    for i in 0..8i32 {
        let cx = ox + (((i >> 2) & 1) * h);
        let cy = oy + (((i >> 1) & 1) * h);
        let cz = oz + ((i & 1) * h);
        if size == 2 {
            let id = ids[block_index(cx, cy, cz)];
            if id != 0 {
                nodes[index + i as usize] = Node::new(CHUNK_OFFSET + id, Voxel::new(0, 0, 0));
            }
        } else if !region_empty(ids, cx, cy, cz, h) {
            let child = build_group(nodes, ids, cx, cy, cz, h);
            nodes[index + i as usize].pointer = child as u32;
        }
    }

    index
}

pub fn generate_block_chunk(cx: i32, cy: i32, cz: i32) -> CpuOctree {
    let mut ids = [0u32; (BLOCKS_PER_CHUNK * BLOCKS_PER_CHUNK * BLOCKS_PER_CHUNK) as usize];
    let origin_x = cx * BLOCKS_PER_CHUNK;
    let origin_y = cy * BLOCKS_PER_CHUNK;
    let origin_z = cz * BLOCKS_PER_CHUNK;
    let mut any = false;
    for y in 0..BLOCKS_PER_CHUNK {
        for x in 0..BLOCKS_PER_CHUNK {
            for z in 0..BLOCKS_PER_CHUNK {
                let id = block_at(origin_x + x, origin_y + y, origin_z + z);
                if id != 0 {
                    any = true;
                    ids[block_index(x, y, z)] = id;
                }
            }
        }
    }

    if !any {
        return CpuOctree::new(0);
    }

    let mut nodes = Vec::new();
    let root = build_group(&mut nodes, &ids, 0, 0, 0, BLOCKS_PER_CHUNK);
    debug_assert_eq!(root, 0);

    CpuOctree {
        nodes,
        top_mip: Voxel::new(0, 0, 0),
    }
}

pub fn spawn_blocks() -> Vector3<f32> {
    let x = 8;
    let mut z = 8;
    while is_tree(x, z) {
        z += 3;
    }
    let h = terrain_height(x, z);
    Vector3::new(x as f32 + 0.5, h as f32 + 18.5, z as f32 - 20.5)
}

pub fn player_start(origin: (i32, i32, i32)) -> (Point3<f32>, Vector3<f32>) {
    let p = blocks_to_octree(spawn_blocks(), origin);
    let pos = Point3::new(p.x, p.y, p.z);
    let look = Vector3::new(0.05, -0.45, 1.0);
    (pos, look)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_continues_through_negative_blocks() {
        let x = -100;
        let z = -40;
        let h = terrain_height(x, z);
        assert_eq!(block_at(x, h, z), GRASS);
        assert_eq!(block_at(x, h - 5, z), STONE);
        assert_eq!(
            chunk_kind(
                x.div_euclid(BLOCKS_PER_CHUNK),
                (h - 40).div_euclid(BLOCKS_PER_CHUNK),
                z.div_euclid(BLOCKS_PER_CHUNK)
            ),
            Some(STONE)
        );
    }

    #[test]
    fn chunk_octree_lands_on_surface_blocks() {
        let x = 40;
        let z = 70;
        let h = terrain_height(x, z);

        let check = |wy: i32, expect: u32| {
            let cx = chunk_of_block(x);
            let cy = chunk_of_block(wy);
            let cz = chunk_of_block(z);
            let chunk = generate_block_chunk(cx, cy, cz);
            let pos = Vector3::new(
                cell_center(x - cx * BLOCKS_PER_CHUNK, BLOCKS_PER_CHUNK),
                cell_center(wy - cy * BLOCKS_PER_CHUNK, BLOCKS_PER_CHUNK),
                cell_center(z - cz * BLOCKS_PER_CHUNK, BLOCKS_PER_CHUNK),
            );
            let (node, depth, _) = chunk.find_voxel(pos, None);
            assert_eq!(depth, CHUNK_DEPTH);
            assert_eq!(chunk.nodes[node].pointer, CHUNK_OFFSET + expect);
            assert_eq!(block_at(x, wy, z), expect);
        };

        check(h, GRASS);
        check(h - 1, DIRT);
        check(h - 5, STONE);
    }

    #[test]
    fn heightfield_march_lands_on_the_sheet_without_skipping() {
        let samples = [
            Vector3::new(0.0, 8000.0, 0.0),
            Vector3::new(-1200.0, 500.0, 400.0),
            Vector3::new(80.0, 200.0, -60.0),
        ];
        let dirs = [
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(0.4, -0.3, 0.15),
            Vector3::new(-0.2, -0.05, 0.9),
        ];
        for origin in samples {
            for dir in dirs {
                let dir = dir.normalize();
                let hit = march_terrain(origin, dir).expect("ray should meet the sheet");
                let h = terrain_surface(hit.x, hit.z);
                assert!((hit.y - h).abs() < 0.08, "hit {hit:?} vs height {h}");
                let t_hit = if dir.z.abs() >= dir.x.abs() && dir.z.abs() > 1e-6 {
                    (hit.z - origin.z) / dir.z
                } else if dir.x.abs() > 1e-6 {
                    (hit.x - origin.x) / dir.x
                } else {
                    (origin.y - hit.y) / -dir.y
                };
                let mut t = 0.0;
                let dt = (t_hit / 48.0).max(1.0);
                while t < t_hit {
                    let sample = origin + dir * t;
                    let sample_h = terrain_surface(sample.x, sample.z);
                    assert!(
                        sample.y >= sample_h - 0.25,
                        "stepped through the sheet at {sample:?}, height {sample_h}"
                    );
                    t += dt;
                }
                let (gx, gz) = terrain_gradient(hit.x, hit.z);
                assert!(gx.abs() <= 0.70 && gz.abs() <= 0.73);
            }
        }

        assert!(march_terrain(Vector3::new(0.0, 5000.0, 0.0), Vector3::new(0.0, 1.0, 0.0)).is_none());
    }
}
