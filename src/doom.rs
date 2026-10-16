use super::*;
use std::collections::HashMap;

/// Doom draws one texture pixel per map unit. The voxel grid uses that same scale.
const TEXEL_SCALE: f32 = 1.0;
/// Eye height and clearance, in texels, so the start sits in a walkable room.
const EYE_HEIGHT: i32 = 41;
const HEADROOM: i32 = 56;
const DOOM_GLB: &[u8] = include_bytes!("../files/doom_e1m1.glb");

struct Tri {
    v: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
    texture: Option<usize>,
    factor: [f32; 4],
}

struct Cell {
    x: i32,
    y: i32,
    z: i32,
    color: Voxel,
}

pub struct LoadedMap {
    pub chunk: CpuOctree,
    pub spawn: Point3<f32>,
    pub look: Vector3<f32>,
    pub grid: i32,
    pub walk: bool,
    pub pictures: crate::play::Pictures,
}

/// Load E1M1 at one texel per voxel. A Doom WAD on disk supplies the map, the
/// pictures, and the things. The hangar mesh is the fallback when no WAD is found.
pub fn load_e1m1() -> LoadedMap {
    if let Some(map) = crate::wad::try_load_e1m1() {
        return finish(
            map.voxels,
            map.spawn,
            map.look,
            map.grid,
            true,
            map.pictures,
        );
    }

    let (tris, images) = read_glb(DOOM_GLB).expect("Doom E1M1 map");
    let (min, max) = bounds(&tris);
    let span = (0..3)
        .map(|k| (max[k] - min[k]).ceil() as i32 + 2)
        .max()
        .unwrap_or(1);
    let grid = (span as u32).next_power_of_two().max(1) as i32;
    let offset = [-min[0], -min[1], -min[2]];
    println!(
        "Voxelizing Doom E1M1 ({} triangles) at 1 texel per voxel into {grid}³",
        tris.len()
    );

    let mut voxels: HashMap<u64, Voxel> = HashMap::new();
    for tri in &tris {
        stamp_triangle(tri, TEXEL_SCALE, offset, grid, &images, &mut voxels);
    }
    let (spawn_voxel, look_grid) = stand_in_the_map(&voxels);
    finish(
        voxels,
        spawn_voxel,
        look_grid,
        grid,
        false,
        crate::play::Pictures::empty(),
    )
}

fn finish(
    voxels: HashMap<u64, Voxel>,
    spawn_voxel: [i32; 3],
    look_grid: [f32; 3],
    grid: i32,
    walk: bool,
    pictures: crate::play::Pictures,
) -> LoadedMap {
    let bits = grid.trailing_zeros() as i32;
    println!("Doom E1M1: {} solid voxels", voxels.len());
    let mut cells: Vec<Cell> = voxels
        .into_iter()
        .map(|(key, color)| {
            let (x, y, z) = decode(key);
            Cell { x, y, z, color }
        })
        .collect();
    cells.sort_by_key(|cell| path_key(cell.x, cell.y, cell.z, bits));

    let mut nodes = Vec::new();
    let root_index = build_range(&cells, bits - 1, &mut nodes);
    debug_assert_eq!(root_index, 0);

    let octree = CpuOctree {
        nodes,
        top_mip: Voxel::new(0, 0, 0),
    };
    let spawn = voxel_center(spawn_voxel, grid);
    LoadedMap {
        chunk: octree,
        spawn: Point3::new(spawn.x, spawn.y, spawn.z),
        look: Vector3::new(look_grid[0], look_grid[1], look_grid[2]),
        grid,
        walk,
        pictures,
    }
}

fn bounds(tris: &[Tri]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for tri in tris {
        for v in tri.v {
            for k in 0..3 {
                min[k] = min[k].min(v[k]);
                max[k] = max[k].max(v[k]);
            }
        }
    }
    (min, max)
}

fn stamp_triangle(
    tri: &Tri,
    scale: f32,
    offset: [f32; 3],
    grid: i32,
    images: &[image::RgbaImage],
    voxels: &mut HashMap<u64, Voxel>,
) {
    let v = [
        to_grid(tri.v[0], scale, offset),
        to_grid(tri.v[1], scale, offset),
        to_grid(tri.v[2], scale, offset),
    ];
    let mut min = [i32::MAX; 3];
    let mut max = [i32::MIN; 3];
    for p in v {
        for k in 0..3 {
            min[k] = min[k].min(p[k].floor() as i32);
            max[k] = max[k].max(p[k].ceil() as i32);
        }
    }
    for k in 0..3 {
        min[k] = min[k].clamp(0, grid - 1);
        max[k] = max[k].clamp(0, grid - 1);
    }

    for z in min[2]..=max[2] {
        for y in min[1]..=max[1] {
            for x in min[0]..=max[0] {
                let p = [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5];
                let Some(bary) = triangle_bary(p, v[0], v[1], v[2]) else {
                    continue;
                };
                let uv = [
                    tri.uv[0][0] * bary[0] + tri.uv[1][0] * bary[1] + tri.uv[2][0] * bary[2],
                    tri.uv[0][1] * bary[0] + tri.uv[1][1] * bary[1] + tri.uv[2][1] * bary[2],
                ];
                let mut rgb = [
                    (tri.factor[0] * 255.0) as u8,
                    (tri.factor[1] * 255.0) as u8,
                    (tri.factor[2] * 255.0) as u8,
                ];
                if let Some(index) = tri.texture {
                    if let Some(img) = images.get(index) {
                        let sample = sample_texture(img, uv);
                        if sample[3] == 0 {
                            continue;
                        }
                        rgb = [
                            (sample[0] as f32 * tri.factor[0]) as u8,
                            (sample[1] as f32 * tri.factor[1]) as u8,
                            (sample[2] as f32 * tri.factor[2]) as u8,
                        ];
                    }
                }
                // A zero colour is air to the raytracer, so a black texel still occupies the cell.
                if rgb == [0, 0, 0] {
                    rgb = [1, 1, 1];
                }
                voxels.insert(pack(x, y, z), Voxel::new(rgb[0], rgb[1], rgb[2]));
            }
        }
    }
}

fn to_grid(p: [f32; 3], scale: f32, offset: [f32; 3]) -> [f32; 3] {
    [
        p[0] * scale + offset[0],
        p[1] * scale + offset[1],
        p[2] * scale + offset[2],
    ]
}

/// Barycentric weights if the voxel centre lies on the triangle, thickened by half a cell
/// along the normal so a flat wall still lands in the grid.
fn triangle_bary(p: [f32; 3], v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> Option<[f32; 3]> {
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let n = cross(e1, e2);
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-6 {
        return None;
    }
    let n = [n[0] / len, n[1] / len, n[2] / len];
    let d = sub(p, v0);
    let dist = (n[0] * d[0] + n[1] * d[1] + n[2] * d[2]).abs();
    let max_dist = 0.5 * (n[0].abs() + n[1].abs() + n[2].abs());
    if dist > max_dist {
        return None;
    }

    let dot00 = dot(e2, e2);
    let dot01 = dot(e2, e1);
    let dot02 = dot(e2, d);
    let dot11 = dot(e1, e1);
    let dot12 = dot(e1, d);
    let denom = dot00 * dot11 - dot01 * dot01;
    if denom.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / denom;
    let u = (dot11 * dot02 - dot01 * dot12) * inv;
    let v = (dot00 * dot12 - dot01 * dot02) * inv;
    if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
        Some([1.0 - u - v, v, u])
    } else {
        None
    }
}

fn sample_texture(img: &image::RgbaImage, uv: [f32; 2]) -> [u8; 4] {
    let w = img.width().max(1);
    let h = img.height().max(1);
    let u = uv[0].rem_euclid(1.0);
    let v = uv[1].rem_euclid(1.0);
    let x = ((u * w as f32) as u32).min(w - 1);
    let y = ((v * h as f32) as u32).min(h - 1);
    img.get_pixel(x, y).0
}

fn stand_in_the_map(voxels: &HashMap<u64, Voxel>) -> ([i32; 3], [f32; 3]) {
    let mut min = [i32::MAX; 3];
    let mut max = [i32::MIN; 3];
    for key in voxels.keys() {
        let (x, y, z) = decode(*key);
        min[0] = min[0].min(x);
        min[1] = min[1].min(y);
        min[2] = min[2].min(z);
        max[0] = max[0].max(x);
        max[1] = max[1].max(y);
        max[2] = max[2].max(z);
    }
    let cx = (min[0] + max[0]) / 2;
    let cz = (min[2] + max[2]) / 2;
    let mut best: Option<[i32; 3]> = None;
    let mut best_d = i32::MAX;
    for need in [HEADROOM, 32, 16] {
        let mut x = min[0];
        while x <= max[0] {
            let mut z = min[2];
            while z <= max[2] {
                let mut saw_floor = false;
                for y in min[1]..=max[1] {
                    if voxels.contains_key(&pack(x, y, z)) {
                        saw_floor = true;
                        continue;
                    }
                    if !saw_floor {
                        continue;
                    }
                    let mut air = 0;
                    let mut yy = y;
                    while yy <= max[1] + need && !voxels.contains_key(&pack(x, yy, z)) {
                        air += 1;
                        yy += 1;
                    }
                    if air >= need {
                        let d = (x - cx).abs() + (z - cz).abs();
                        if d < best_d {
                            best_d = d;
                            let eye = y + EYE_HEIGHT.min(air - 8);
                            best = Some([x, eye, z]);
                        }
                    }
                    break;
                }
                z += 16;
            }
            x += 16;
        }
        if best.is_some() {
            break;
        }
    }

    let spot = best.unwrap_or([cx, (min[1] + max[1]) / 2, cz]);
    let dx = (cx - spot[0]) as f32;
    let dz = (cz - spot[2]) as f32;
    let look = if dx.abs() + dz.abs() < 32.0 {
        [1.0, -0.15, 0.1]
    } else {
        let horiz = (dx * dx + dz * dz).sqrt().max(1.0);
        [dx, -0.15 * horiz, dz]
    };
    (spot, look)
}

fn path_key(x: i32, y: i32, z: i32, bits: i32) -> u64 {
    let mut key = 0u64;
    for shift in (0..bits).rev() {
        key = (key << 3) | child_index(x, y, z, shift as u32) as u64;
    }
    key
}

fn build_range(cells: &[Cell], shift: i32, nodes: &mut Vec<Node>) -> usize {
    let index = nodes.len();
    for _ in 0..8 {
        nodes.push(Node::new(CHUNK_OFFSET, Voxel::new(0, 0, 0)));
    }
    let mut start = 0;
    while start < cells.len() {
        let child = child_index(cells[start].x, cells[start].y, cells[start].z, shift as u32);
        let mut end = start + 1;
        while end < cells.len()
            && child_index(cells[end].x, cells[end].y, cells[end].z, shift as u32) == child
        {
            end += 1;
        }
        if shift == 0 {
            nodes[index + child] = Node::new(CHUNK_OFFSET, cells[start].color);
        } else {
            let child_at = build_range(&cells[start..end], shift - 1, nodes);
            nodes[index + child].pointer = child_at as u32;
        }
        start = end;
    }
    index
}

fn voxel_center(v: [i32; 3], grid: i32) -> Vector3<f32> {
    let n = grid as f32;
    Vector3::new(
        -1.0 + (v[0] as f32 + 0.5) * 2.0 / n,
        -1.0 + (v[1] as f32 + 0.5) * 2.0 / n,
        -1.0 + (v[2] as f32 + 0.5) * 2.0 / n,
    )
}

fn world_to_voxel(p: Point3<f32>, grid: i32) -> [f32; 3] {
    let n = grid as f32;
    [
        (p.x + 1.0) * 0.5 * n - 0.5,
        (p.y + 1.0) * 0.5 * n - 0.5,
        (p.z + 1.0) * 0.5 * n - 0.5,
    ]
}

fn voxel_to_world(v: [f32; 3], grid: i32) -> Point3<f32> {
    let n = grid as f32;
    Point3::new(
        -1.0 + (v[0] + 0.5) * 2.0 / n,
        -1.0 + (v[1] + 0.5) * 2.0 / n,
        -1.0 + (v[2] + 0.5) * 2.0 / n,
    )
}

const PLAYER_EYE: f32 = 41.0;
const STEP_UP: i32 = 24;
const BODY_RADIUS: f32 = 16.0;
const GRAVITY: f32 = 1225.0;
const JUMP_SPEED: f32 = 280.0;
const BODY_CLEAR: f32 = 0.25;

/// Leaf centers sit on integers, so the cell covering `p` is the nearest index.
fn cell_at(p: f32) -> i32 {
    (p + 0.5).floor() as i32
}

fn cell_solid(octree: &CpuOctree, grid: i32, x: i32, y: i32, z: i32) -> bool {
    if x < 0 || y < 0 || z < 0 || x >= grid || y >= grid || z >= grid {
        return true;
    }
    let p = voxel_center([x, y, z], grid);
    let (node, _, _) = octree.find_voxel(Vector3::new(p.x, p.y, p.z), None);
    octree.nodes[node].value != Voxel::new(0, 0, 0)
}

fn body_offsets() -> [(f32, f32); 9] {
    let r = BODY_RADIUS;
    [
        (0.0, 0.0),
        (r, 0.0),
        (-r, 0.0),
        (0.0, r),
        (0.0, -r),
        (r * 0.7, r * 0.7),
        (r * 0.7, -r * 0.7),
        (-r * 0.7, r * 0.7),
        (-r * 0.7, -r * 0.7),
    ]
}

/// Closest floor surface to the feet, in a window that includes one stair up.
fn surface_near(octree: &CpuOctree, grid: i32, x: f32, z: f32, feet: f32) -> Option<i32> {
    let ix = cell_at(x);
    let iz = cell_at(z);
    let top = cell_at(feet + STEP_UP as f32);
    let bot = cell_at(feet - 96.0);
    let mut best: Option<i32> = None;
    let mut best_dist = f32::MAX;
    for y in (bot..=top).rev() {
        if !cell_solid(octree, grid, ix, y, iz) || cell_solid(octree, grid, ix, y + 1, iz) {
            continue;
        }
        let surface = y as f32 + 0.5;
        let rise = surface - feet;
        if rise > STEP_UP as f32 || rise < -96.0 {
            continue;
        }
        let dist = rise.abs();
        if dist < best_dist {
            best_dist = dist;
            best = Some(y);
        }
    }
    best
}

/// A column blocks the body when it is taller than a stair. A rise of at most
/// 24 with air above it is a step, and the body may overlap it.
fn column_blocks(octree: &CpuOctree, grid: i32, x: i32, z: i32, floor: i32) -> bool {
    let mut y = floor + 1;
    // Stop short of a 56-unit ceiling slab. A real wall is solid the whole way up.
    let limit = floor + 50;
    while y <= limit {
        if !cell_solid(octree, grid, x, y, z) {
            y += 1;
            continue;
        }
        let mut top = y;
        while top <= limit && cell_solid(octree, grid, x, top, z) {
            top += 1;
        }
        if top > limit {
            return true;
        }
        if (top - 1) - floor > STEP_UP {
            return true;
        }
        y = top;
    }
    false
}

fn hits_wall(octree: &CpuOctree, grid: i32, eye: [f32; 3], floor: i32) -> bool {
    body_offsets().iter().any(|(ox, oz)| {
        column_blocks(
            octree,
            grid,
            cell_at(eye[0] + ox),
            cell_at(eye[2] + oz),
            floor,
        )
    })
}

fn body_solid(octree: &CpuOctree, grid: i32, eye: [f32; 3]) -> bool {
    let feet = eye[1] - PLAYER_EYE;
    let heights = [feet + 8.0, feet + 28.0, feet + 48.0];
    for (ox, oz) in body_offsets() {
        for y in heights {
            if cell_solid(
                octree,
                grid,
                cell_at(eye[0] + ox),
                cell_at(y),
                cell_at(eye[2] + oz),
            ) {
                return true;
            }
        }
    }
    false
}

pub fn fits_body(octree: &CpuOctree, grid: i32, pos: Point3<f32>) -> bool {
    !body_solid(octree, grid, world_to_voxel(pos, grid))
}

fn try_move(
    octree: &CpuOctree,
    grid: i32,
    eye: [f32; 3],
    dx: f32,
    dz: f32,
    on_ground: bool,
) -> Option<[f32; 3]> {
    let mut next = eye;
    next[0] += dx;
    next[2] += dz;
    let feet = eye[1] - PLAYER_EYE;
    if on_ground {
        if let Some(floor) = surface_near(octree, grid, next[0], next[2], feet) {
            let surface = floor as f32 + 0.5;
            let rise = surface - feet;
            if rise <= STEP_UP as f32 && rise >= -(STEP_UP as f32) {
                next[1] = surface + PLAYER_EYE + BODY_CLEAR;
                if !hits_wall(octree, grid, next, floor) {
                    return Some(next);
                }
                return None;
            }
            if rise < -(STEP_UP as f32) && !body_solid(octree, grid, next) {
                return Some(next);
            }
            return None;
        }
    }
    if body_solid(octree, grid, next) {
        None
    } else {
        Some(next)
    }
}

fn slide(
    octree: &CpuOctree,
    grid: i32,
    eye: [f32; 3],
    dx: f32,
    dz: f32,
    on_ground: bool,
) -> [f32; 3] {
    if let Some(moved) = try_move(octree, grid, eye, dx, dz, on_ground) {
        return moved;
    }
    if dx.abs() > 1.0e-6 {
        if let Some(moved) = try_move(octree, grid, eye, dx, 0.0, on_ground) {
            return moved;
        }
    }
    if dz.abs() > 1.0e-6 {
        if let Some(moved) = try_move(octree, grid, eye, 0.0, dz, on_ground) {
            return moved;
        }
    }
    eye
}

/// Walk, run, and jump against the voxel map. `wish` is a horizontal direction
/// in world space. Speed is in map units per second, which is one voxel.
pub fn step_on_foot(
    octree: &CpuOctree,
    grid: i32,
    pos: Point3<f32>,
    wish: Vector3<f32>,
    speed: f32,
    mut vy: f32,
    dt: f32,
    jump: bool,
    mut on_ground: bool,
) -> (Point3<f32>, f32, bool) {
    let mut eye = world_to_voxel(pos, grid);
    let dx = wish.x * speed * dt;
    let dz = wish.z * speed * dt;
    let dist = (dx * dx + dz * dz).sqrt();
    let sub = (dist / 4.0).ceil().max(1.0) as i32;
    if dist > 1e-6 {
        let sx = dx / sub as f32;
        let sz = dz / sub as f32;
        for _ in 0..sub {
            eye = slide(octree, grid, eye, sx, sz, on_ground);
        }
    }

    if jump && on_ground {
        vy = JUMP_SPEED;
        on_ground = false;
    }

    let feet = eye[1] - PLAYER_EYE;
    if on_ground {
        match surface_near(octree, grid, eye[0], eye[2], feet) {
            Some(floor) if (floor as f32 + 0.5 - feet).abs() <= STEP_UP as f32 + 1.0 => {
                eye[1] = floor as f32 + 0.5 + PLAYER_EYE + BODY_CLEAR;
                vy = 0.0;
            }
            _ => on_ground = false,
        }
    }
    if !on_ground {
        vy = (vy - GRAVITY * dt).max(-2400.0);
        let dy = vy * dt;
        let steps = (dy.abs() / 4.0).ceil().max(1.0) as i32;
        let step = dy / steps as f32;
        for _ in 0..steps {
            let mut moved = eye;
            moved[1] += step;
            if step < 0.0 {
                if let Some(floor) = surface_near(octree, grid, eye[0], eye[2], eye[1] - PLAYER_EYE)
                {
                    let surface = floor as f32 + 0.5;
                    if moved[1] - PLAYER_EYE <= surface + BODY_CLEAR {
                        eye[1] = surface + PLAYER_EYE + BODY_CLEAR;
                        on_ground = true;
                        vy = 0.0;
                        break;
                    }
                }
            }
            if body_solid(octree, grid, moved) {
                vy = 0.0;
                break;
            }
            eye = moved;
        }
    }

    (voxel_to_world(eye, grid), vy, on_ground)
}

/// World distance to the first solid voxel along a unit direction.
pub fn wall_distance(octree: &CpuOctree, grid: i32, eye: Point3<f32>, dir: Vector3<f32>) -> f32 {
    let scale = grid as f32 * 0.5;
    let origin = world_to_voxel(eye, grid);
    let delta = [dir.x * scale, dir.y * scale, dir.z * scale];
    if delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2] < 1.0e-12 {
        return 1.0e6;
    }
    let mut cell = [cell_at(origin[0]), cell_at(origin[1]), cell_at(origin[2])];
    let mut t_max = [f32::MAX; 3];
    let mut t_delta = [f32::MAX; 3];
    let mut step = [0; 3];
    for axis in 0..3 {
        if delta[axis].abs() < 1.0e-8 {
            continue;
        }
        step[axis] = if delta[axis] > 0.0 { 1 } else { -1 };
        let edge = if delta[axis] > 0.0 {
            cell[axis] as f32 + 0.5
        } else {
            cell[axis] as f32 - 0.5
        };
        t_max[axis] = (edge - origin[axis]) / delta[axis];
        t_delta[axis] = (1.0 / delta[axis]).abs();
    }
    let mut t = 0.0;
    for _ in 0..8000 {
        if t > 3.0 / grid as f32 && cell_solid(octree, grid, cell[0], cell[1], cell[2]) {
            return t;
        }
        let axis = if t_max[0] < t_max[1] && t_max[0] < t_max[2] {
            0
        } else if t_max[1] < t_max[2] {
            1
        } else {
            2
        };
        if t_max[axis] > 4000.0 {
            break;
        }
        t = t_max[axis];
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
    }
    1.0e6
}

fn pack(x: i32, y: i32, z: i32) -> u64 {
    (x as u64) | ((y as u64) << 16) | ((z as u64) << 32)
}

fn decode(key: u64) -> (i32, i32, i32) {
    (
        (key & 0xffff) as i32,
        ((key >> 16) & 0xffff) as i32,
        ((key >> 32) & 0xffff) as i32,
    )
}

fn child_index(x: i32, y: i32, z: i32, shift: u32) -> usize {
    let xi = ((x >> shift) & 1) as usize;
    let yi = ((y >> shift) & 1) as usize;
    let zi = ((z >> shift) & 1) as usize;
    xi * 4 + yi * 2 + zi
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn read_glb(bytes: &[u8]) -> Result<(Vec<Tri>, Vec<image::RgbaImage>), String> {
    let (json_bytes, blob) = glb_chunks(bytes)?;
    let json = std::str::from_utf8(json_bytes).map_err(|e| e.to_string())?;
    let doc = json::parse(json).map_err(|e| e.to_string())?;

    let mut images = Vec::new();
    for image in doc["images"].members() {
        if image["bufferView"].is_null() {
            images.push(image::RgbaImage::new(1, 1));
            continue;
        }
        let png = view_bytes(&doc, blob, usize_of(&image["bufferView"])?)?;
        let decoded = image::load_from_memory(png)
            .map_err(|e| e.to_string())?
            .to_rgba8();
        images.push(decoded);
    }

    let mut tris = Vec::new();
    for scene in doc["scenes"].members() {
        for node in scene["nodes"].members() {
            collect_node(&doc, blob, usize_of(node)?, identity(), &mut tris)?;
        }
    }
    if tris.is_empty() {
        return Err("Doom map has no triangles".into());
    }
    Ok((tris, images))
}

fn collect_node(
    doc: &json::JsonValue,
    blob: &[u8],
    index: usize,
    parent: [[f32; 4]; 4],
    tris: &mut Vec<Tri>,
) -> Result<(), String> {
    let node = &doc["nodes"][index];
    let local = if node["matrix"].is_null() {
        identity()
    } else {
        let mut m = [0.0; 16];
        for (i, value) in node["matrix"].members().enumerate() {
            m[i] = f32_of(value)?;
        }
        mat_from_gltf(m)
    };
    let transform = mul_mat(parent, local);
    if !node["mesh"].is_null() {
        let mesh = &doc["meshes"][usize_of(&node["mesh"])?];
        for primitive in mesh["primitives"].members() {
            push_primitive(doc, blob, primitive, transform, tris)?;
        }
    }
    for child in node["children"].members() {
        collect_node(doc, blob, usize_of(child)?, transform, tris)?;
    }
    Ok(())
}

fn push_primitive(
    doc: &json::JsonValue,
    blob: &[u8],
    primitive: &json::JsonValue,
    transform: [[f32; 4]; 4],
    tris: &mut Vec<Tri>,
) -> Result<(), String> {
    let material = if primitive["material"].is_null() {
        None
    } else {
        Some(&doc["materials"][usize_of(&primitive["material"])?])
    };
    if material.and_then(|m| m["name"].as_str()) == Some("F_SKY1") {
        return Ok(());
    }
    let pbr = material.map(|m| &m["pbrMetallicRoughness"]);
    let factor = if let Some(pbr) = pbr {
        if pbr["baseColorFactor"].is_null() {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            let mut factor = [1.0; 4];
            for (i, value) in pbr["baseColorFactor"].members().enumerate().take(4) {
                factor[i] = f32_of(value)?;
            }
            factor
        }
    } else {
        [1.0, 1.0, 1.0, 1.0]
    };
    if factor[3] == 0.0 {
        return Ok(());
    }
    let texture = if let Some(pbr) = pbr {
        if pbr["baseColorTexture"].is_null() {
            None
        } else {
            let index = usize_of(&pbr["baseColorTexture"]["index"])?;
            if doc["textures"][index]["source"].is_null() {
                None
            } else {
                Some(usize_of(&doc["textures"][index]["source"])?)
            }
        }
    } else {
        None
    };

    if primitive["attributes"]["POSITION"].is_null() {
        return Ok(());
    }
    let positions = read_vec3(doc, blob, usize_of(&primitive["attributes"]["POSITION"])?)?
        .into_iter()
        .map(|p| mul_point(transform, p))
        .collect::<Vec<_>>();
    let uvs = if primitive["attributes"]["TEXCOORD_0"].is_null() {
        Vec::new()
    } else {
        read_vec2(doc, blob, usize_of(&primitive["attributes"]["TEXCOORD_0"])?)?
    };
    if primitive["indices"].is_null() {
        return Ok(());
    }
    let indices = read_indices(doc, blob, usize_of(&primitive["indices"])?)?;

    for corner in indices.chunks_exact(3) {
        let i0 = corner[0] as usize;
        let i1 = corner[1] as usize;
        let i2 = corner[2] as usize;
        let uv = |i: usize| uvs.get(i).copied().unwrap_or([0.0, 0.0]);
        tris.push(Tri {
            v: [positions[i0], positions[i1], positions[i2]],
            uv: [uv(i0), uv(i1), uv(i2)],
            texture,
            factor,
        });
    }
    Ok(())
}

fn usize_of(value: &json::JsonValue) -> Result<usize, String> {
    value
        .as_u64()
        .map(|n| n as usize)
        .ok_or_else(|| "expected an index".into())
}

fn f32_of(value: &json::JsonValue) -> Result<f32, String> {
    value
        .as_f64()
        .map(|n| n as f32)
        .ok_or_else(|| "expected a float".into())
}

fn glb_chunks(bytes: &[u8]) -> Result<(&[u8], &[u8]), String> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err("not a glb".into());
    }
    let mut off = 12;
    let mut json = None;
    let mut bin = None;
    while off + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
        let kind = &bytes[off + 4..off + 8];
        off += 8;
        let chunk = bytes.get(off..off + len).ok_or("truncated glb chunk")?;
        off += len;
        if kind == b"JSON" {
            json = Some(chunk);
        } else if kind == b"BIN\0" {
            bin = Some(chunk);
        }
    }
    Ok((
        json.ok_or("glb has no JSON")?,
        bin.ok_or("glb has no buffer")?,
    ))
}

fn view_bytes<'a>(doc: &json::JsonValue, blob: &'a [u8], index: usize) -> Result<&'a [u8], String> {
    let view = &doc["bufferViews"][index];
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    let len = usize_of(&view["byteLength"])?;
    blob.get(start..start + len)
        .ok_or_else(|| "buffer view is outside the buffer".into())
}

fn read_vec3(doc: &json::JsonValue, blob: &[u8], index: usize) -> Result<Vec<[f32; 3]>, String> {
    let (data, stride, count) = accessor_bytes(doc, blob, index, 12)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = i * stride;
        out.push([
            f32::from_le_bytes(data[o..o + 4].try_into().unwrap()),
            f32::from_le_bytes(data[o + 4..o + 8].try_into().unwrap()),
            f32::from_le_bytes(data[o + 8..o + 12].try_into().unwrap()),
        ]);
    }
    Ok(out)
}

fn read_vec2(doc: &json::JsonValue, blob: &[u8], index: usize) -> Result<Vec<[f32; 2]>, String> {
    let (data, stride, count) = accessor_bytes(doc, blob, index, 8)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = i * stride;
        out.push([
            f32::from_le_bytes(data[o..o + 4].try_into().unwrap()),
            f32::from_le_bytes(data[o + 4..o + 8].try_into().unwrap()),
        ]);
    }
    Ok(out)
}

fn read_indices(doc: &json::JsonValue, blob: &[u8], index: usize) -> Result<Vec<u32>, String> {
    let accessor = &doc["accessors"][index];
    let component = usize_of(&accessor["componentType"])?;
    let width = match component {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        other => return Err(format!("unsupported index type {other}")),
    };
    let (data, stride, count) = accessor_bytes(doc, blob, index, width)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = i * stride;
        let value = match width {
            1 => data[o] as u32,
            2 => u16::from_le_bytes(data[o..o + 2].try_into().unwrap()) as u32,
            _ => u32::from_le_bytes(data[o..o + 4].try_into().unwrap()),
        };
        out.push(value);
    }
    Ok(out)
}

fn accessor_bytes<'a>(
    doc: &json::JsonValue,
    blob: &'a [u8],
    index: usize,
    tightly: usize,
) -> Result<(&'a [u8], usize, usize), String> {
    let accessor = &doc["accessors"][index];
    let view_index = usize_of(&accessor["bufferView"])?;
    let view = &doc["bufferViews"][view_index];
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize
        + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
    let stride = view["byteStride"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(tightly);
    let count = usize_of(&accessor["count"])?;
    let data = blob
        .get(start..start + count * stride)
        .ok_or("accessor is outside the buffer")?;
    Ok((data, stride, count))
}

fn mat_from_gltf(m: [f32; 16]) -> [[f32; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = m[col * 4 + row];
        }
    }
    out
}

fn identity() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn mul_mat(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut c = [[0.0; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            c[col][row] = a[0][row] * b[col][0]
                + a[1][row] * b[col][1]
                + a[2][row] * b[col][2]
                + a[3][row] * b[col][3];
        }
    }
    c
}

fn mul_point(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    let x = m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0];
    let y = m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1];
    let z = m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2];
    let w = m[0][3] * p[0] + m[1][3] * p[1] + m[2][3] * p[2] + m[3][3];
    [x / w, y / w, z / w]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e1m1_fills_a_finite_cube_and_has_a_floor_under_the_start() {
        let map = load_e1m1();
        let chunk = map.chunk;
        let spawn = map.spawn;
        assert!(chunk.nodes.len() > 8);
        assert!(spawn.x.abs() < 1.0 && spawn.y.abs() < 1.0 && spawn.z.abs() < 1.0);

        let mut found = false;
        let step = 2.0 / map.grid as f32;
        for i in 1..EYE_HEIGHT + 16 {
            let floor = Vector3::new(spawn.x, spawn.y - i as f32 * step, spawn.z);
            let (node, depth, _) = chunk.find_voxel(floor, None);
            if chunk.nodes[node].value != Voxel::new(0, 0, 0) {
                assert_eq!(
                    depth,
                    map.grid.trailing_zeros(),
                    "a texel-sized voxel fills one leaf of the cube"
                );
                found = true;
                break;
            }
        }
        assert!(found, "start should be standing over the map");
        if map.walk {
            assert!(
                map.pictures.planes.len() > 50,
                "things should be sprite planes"
            );
            assert!(
                !map.pictures.guns.is_empty(),
                "the pistol sprite should load"
            );
            assert!(
                map.pictures.guns[0].frames.len() > 1,
                "the pistol should have its firing frames"
            );
            assert!(
                map.pictures.names.contains_key("BEXPA"),
                "barrel explosions need the BEXP frames"
            );
            assert!(
                map.pictures.doors.len() >= 8,
                "each closed door keeps a plane on both faces, got {}",
                map.pictures.doors.len()
            );
            assert!(
                fits_body(&chunk, map.grid, spawn),
                "the player start should be clear of the floor and the ceiling"
            );
            let (next, _, ground) = step_on_foot(
                &chunk,
                map.grid,
                spawn,
                Vector3::new(map.look.x, 0.0, map.look.z),
                32.0,
                0.0,
                0.05,
                false,
                true,
            );
            assert!(ground, "a step from the start stays on the floor");
            assert!(
                (next.y - spawn.y).abs() < step * 4.0,
                "walking should not drop through the floor"
            );
        }

        let mut world = World::bare(String::new());
        world.chunks.insert(0, chunk);
        world.generate_mip_tree(0);
        assert_ne!(world.chunks.get(&0).unwrap().top_mip, Voxel::new(0, 0, 0));
    }

    #[test]
    fn walks_up_a_step_and_stops_at_a_tall_wall() {
        let grid = 128;
        let depth = 7;
        let mut tree = CpuOctree::new(0);
        let solid = Voxel::new(80, 80, 80);
        for x in 8..56 {
            for z in 28..36 {
                let top = if x >= 28 { 20 } else { 4 };
                for y in 0..=top {
                    let p = voxel_center([x, y, z], grid);
                    tree.put_in_voxel(Vector3::new(p.x, p.y, p.z), solid, depth);
                }
            }
        }
        for y in 5..60 {
            for z in 28..36 {
                let p = voxel_center([52, y, z], grid);
                tree.put_in_voxel(Vector3::new(p.x, p.y, p.z), solid, depth);
            }
        }
        assert!(cell_solid(&tree, grid, 12, 4, 32));
        assert!(!cell_solid(&tree, grid, 12, 8, 32));
        assert!(cell_solid(&tree, grid, 32, 20, 32));

        let eye_y = 4.5 + BODY_CLEAR + PLAYER_EYE;
        let mut pos = voxel_to_world([16.0, eye_y, 32.0], grid);
        let mut vy = 0.0;
        let mut ground = true;
        for _ in 0..40 {
            let stepped = step_on_foot(
                &tree,
                grid,
                pos,
                Vector3::new(1.0, 0.0, 0.0),
                200.0,
                vy,
                0.05,
                false,
                ground,
            );
            pos = stepped.0;
            vy = stepped.1;
            ground = stepped.2;
        }
        let eye = world_to_voxel(pos, grid);
        assert!(eye[0] > 30.0, "crossed onto the step, x={}", eye[0]);
        assert!(
            eye[1] > eye_y + 10.0,
            "climbed the 16-unit step, y={}",
            eye[1]
        );
        assert!(eye[0] < 40.0, "the tall wall stops the body, x={}", eye[0]);
        assert!(ground, "standing on the step");
    }
}
