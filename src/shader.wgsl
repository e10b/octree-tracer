// Should be same as main.rs:Unifroms
struct Uniforms {
    camera: mat4x4<f32>;
    camera_inverse: mat4x4<f32>;
    dimensions: vec4<f32>;
    sun_dir: vec4<f32>;
    pause_adaptive: bool;
    show_steps: bool;
    show_hits: bool;
    shadows: bool;
    misc_value: f32;
    misc_bool: bool;
};

struct U32s {
    data: [[stride(4)]] array<u32>;
};

struct AtomicU32s {
    counter: atomic<u32>;
    data: [[stride(4)]] array<u32>;
};

struct HistoryPixel {
    // radiance.xyz is demodulated illumination. w is the accepted history length.
    radiance: vec4<f32>;
    position: vec4<f32>;
    // normal.w is the primary-ray hit distance, used by the depth edge stop.
    normal: vec4<f32>;
    // moments.xy are the first and second raw moments of illumination luminance.
    moments: vec4<f32>;
    albedo: vec4<f32>;
};

struct History {
    data: array<HistoryPixel>;
};

struct PathUniforms {
    frame_index: u32;
    max_bounces: u32;
    texture_emission: f32;
    temporal_history: f32;
    history_valid: u32;
    accumulate: u32;
    still_frames: u32;
    // 1: replace diffuse lighting with the voxel's running mean.
    voxel_light: u32;
    light_scale: u32;
    light_samples: u32;
    normal_scale: f32;
    roughness_scale: f32;
    previous_camera: mat4x4<f32>;
};

[[group(0), binding(0)]]
var<uniform> u: Uniforms; // uniforms
[[group(0), binding(1)]]
var<storage, read_write> n: U32s; // nodes
// read_write, not read: Metal puts read-only storage in the constant address
// space, which is too small for a full-frame history buffer.
[[group(0), binding(2)]]
var<storage, read_write> history_in: History;
[[group(0), binding(3)]]
var<uniform> path: PathUniforms;
[[group(0), binding(4)]]
var<storage, read_write> history_out: History;
[[group(0), binding(5)]]
var blue_noise: texture_2d<f32>;
// One record per node: rgb is the float irradiance mean, w is the sample count.
struct VoxelLight {
    data: [[stride(16)]] array<vec4<f32>>;
};
[[group(0), binding(6)]]
var<storage, read_write> voxel_light: VoxelLight;

var<private> light_leaf: array<u32, 6>;
var<private> light_has: array<u32, 6>;
var<private> light_direct: array<vec3<f32>, 6>;
var<private> light_emit: array<vec3<f32>, 6>;
var<private> light_albedo: array<vec3<f32>, 6>;
var<private> light_spec: array<vec3<f32>, 6>;
var<private> light_indirect: array<f32, 6>;
var<private> light_added: array<vec3<f32>, 6>;


let VOXEL_OFFSET = 134217728u;
// Block span of the traced cube. Matches WINDOW_CHUNKS * BLOCKS_PER_CHUNK.
let WINDOW_SPAN = 128.0;
// Euclidean bound on the slope of terrain_surface. Matches TERRAIN_MAX_SLOPE.
let TERRAIN_MAX_SLOPE = 1.05;
// Highest crest: 72 + 6 + 6 + 14 + 10. Matches TERRAIN_MAX_HEIGHT.
let TERRAIN_MAX_HEIGHT = 108.0;

[[stage(vertex)]]
fn vs_main([[builtin(vertex_index)]] in_vertex_index: u32) -> [[builtin(position)]] vec4<f32> {
    var x = 0.0;
    var y = 0.0;

    if (in_vertex_index == 0u) {
        x = -1.0;
        y = -1.0;
    } else if (in_vertex_index == 1u) {
        x = 1.0;
        y = -1.0;
    } else if (in_vertex_index == 2u) {
        x = -1.0;
        y = 1.0;
    } else if (in_vertex_index == 3u) {
        x = 1.0;
        y = 1.0;
    }

    return vec4<f32>(x, y, 0.0, 1.0);
}

fn get_clip_space(frag_pos: vec4<f32>, dimensions: vec2<f32>) -> vec2<f32> {
    var clip_space = frag_pos.xy / dimensions * 2.0;
    clip_space = clip_space - 1.0;
    clip_space = clip_space * vec2<f32>(1.0, -1.0);
    return clip_space;
}

struct Ray {
    pos: vec3<f32>;
    dir: vec3<f32>;
};

fn ray_box_dist(r: Ray, vmin: vec3<f32>, vmax: vec3<f32>) -> f32 {
    let v1 = (vmin.x - r.pos.x) / r.dir.x;
    let v2 = (vmax.x - r.pos.x) / r.dir.x;
    let v3 = (vmin.y - r.pos.y) / r.dir.y;
    let v4 = (vmax.y - r.pos.y) / r.dir.y;
    let v5 = (vmin.z - r.pos.z) / r.dir.z;
    let v6 = (vmax.z - r.pos.z) / r.dir.z;
    let v7 = max(max(min(v1, v2), min(v3, v4)), min(v5, v6));
    let v8 = min(min(max(v1, v2), max(v3, v4)), max(v5, v6));
    if (v8 < 0.0 || v7 > v8) {
        return 0.0;
    }
    
    return v7;
}

struct FSIn {
    [[builtin(position)]] frag_pos: vec4<f32>;
};


fn count_bits(n: u32) -> u32 {
    var count = 0u;
    var n = n;
    loop {
        if (n == 0u) { break; }
        n = n & (n - 1u);
        count = count + 1u;
    }
    return count;
}

fn unpack_u8(p: u32) -> vec4<u32> {
    return vec4<u32>(
        (p >> 24u) & 0xFFu,
        (p >> 16u) & 0xFFu,
        (p >> 8u) & 0xFFu,
        p & 0xFFu
    );
}

fn unpack_u24u8(p: u32) -> vec2<u32> {
    return vec2<u32>(
        (p >> 8u) & 0xFFFFFFu,
        p & 0xFFu
    );
}

struct Node {
    mask: u32;
    index: u32;
};

fn node(i: u32) -> u32 {
    return n.data[i] >> 4u;
}

struct Voxel {
    value: u32;
    pos: vec3<f32>;
    depth: u32;
};

// Returns leaf containing position
fn find_voxel(pos: vec3<f32>, primary: bool) -> Voxel {
    var node_index = 0u;
    var node_pos = vec3<f32>(0.0);
    var depth = 0u;
    loop {
        depth = depth + 1u;

        var p: vec3<u32>;
        if (u.misc_bool) {
            p = vec3<u32>(
                u32(pos.x >= node_pos.x),
                u32(pos.y >= node_pos.y),
                u32(pos.z >= node_pos.z)
            );
        } else {
            p = vec3<u32>(
                u32(pos.x > node_pos.x),
                u32(pos.y > node_pos.y),
                u32(pos.z > node_pos.z)
            );
        }
        let child_index = p.x * 4u + p.y * 2u + p.z;

        node_pos = node_pos + (vec3<f32>(p) * 2.0 - 1.0) / f32(1u << depth);

        let p = node_index + child_index;

        // Increment counters
        let value = n.data[p];
        if (primary && (value & 15u) < 15u && !u.pause_adaptive) {
            n.data[p] = value + 1u;
        }

        // tnipt: thing node is pointing to ;)
        // Lets be honest, you dont know how to name variables either
        let tnipt = node(p);
        if (tnipt >= VOXEL_OFFSET) {
            return Voxel(p, node_pos, depth);
        }

        node_index = tnipt;
    }

    // Should never get here
    return Voxel(0u, vec3<f32>(0.0), 0u);
}

fn in_bounds(v: vec3<f32>) -> bool {
    let s = step(vec3<f32>(-1.0), v) - step(vec3<f32>(1.0), v);
    return (s.x * s.y * s.z) > 0.5; 
}

struct HitInfo {
    hit: bool;
    value: u32;
    pos: vec3<f32>;
    normal: vec3<f32>;
    steps: u32;
    depth: u32;
};

fn octree_ray(r: Ray, primary: bool) -> HitInfo {
    var pos = r.pos;
    let dir_mask = vec3<f32>(r.dir == vec3<f32>(0.0));
    var dir = r.dir + dir_mask * 0.000001;

    var dist = 0.0;
    if (!in_bounds(r.pos)) {
        // Get position on surface of the octree
        dist = ray_box_dist(r, vec3<f32>(-1.0), vec3<f32>(1.0));
        if (dist == 0.0){
            return HitInfo(false, 0u, r.pos, vec3<f32>(0.0), 0u, 0u);
        }

        pos = r.pos + dir * dist;
    }

    let r_sign = sign(dir);

    var voxel = Voxel(0u, vec3<f32>(0.0), 0u);
    var voxel_pos = pos;
    var steps = 0u;
    var normal = trunc(pos * 1.000001);
    loop {
        voxel = find_voxel(voxel_pos, primary);
        if (!u.pause_adaptive || !u.show_hits) {
            let tnipt = node(voxel.value) - VOXEL_OFFSET;
            if (tnipt > 0u) {
                break;
            }
        } else {
            let value = n.data[voxel.value];
            if ((value & 15u) > 0u) {
                break;
            }
        }

        let voxel_size = 2.0 / f32(1u << voxel.depth);
        let t_max = (voxel.pos - pos + r_sign * voxel_size / 2.0) / dir;

        // https://www.shadertoy.com/view/4dX3zl (good old shader toy)
        let mask = vec3<f32>(t_max.xyz <= min(t_max.yzx, t_max.zxy));
        normal = mask * -r_sign;

        let t_current = min(min(t_max.x, t_max.y), t_max.z);
        voxel_pos = pos + dir * t_current - normal * 0.000002;

        if (!in_bounds(voxel_pos)) {
            return HitInfo(false, 0u, voxel_pos, normal, steps, voxel.depth);
        }

        steps = steps + 1u;
        if (steps > 100u) {
            return HitInfo(true, 0xFF000000u, voxel_pos, normal, steps, 100u);
        }
    }

    return HitInfo(true, voxel.value, voxel_pos, normal, steps, voxel.depth);
}

fn terrain_enabled() -> bool {
    // sun_dir.w is NaN when the height sheet should stay off (a loaded model, not the block world).
    return u.sun_dir.w == u.sun_dir.w;
}

fn window_origin_blocks() -> vec3<f32> {
    return vec3<f32>(u.dimensions.z, u.sun_dir.w, u.dimensions.w);
}

fn octree_to_blocks(p: vec3<f32>) -> vec3<f32> {
    return (p + vec3<f32>(1.0)) * (WINDOW_SPAN * 0.5) + window_origin_blocks();
}

fn blocks_to_octree(b: vec3<f32>) -> vec3<f32> {
    return (b - window_origin_blocks()) / WINDOW_SPAN * 2.0 - vec3<f32>(1.0);
}

// Same formula as terrain_surface in block_world.rs.
fn terrain_surface(x: f32, z: f32) -> f32 {
    return 72.0
        + sin(x * 0.05) * 6.0
        + cos(z * 0.05) * 6.0
        + sin((x + z) * 0.02) * 14.0
        + cos(x * 0.01 - z * 0.013) * 10.0;
}

// (dh/dx, dh/dz). Same formula as terrain_gradient in block_world.rs.
fn terrain_gradient(x: f32, z: f32) -> vec2<f32> {
    let a = x * 0.05;
    let b = z * 0.05;
    let c = (x + z) * 0.02;
    let d = x * 0.01 - z * 0.013;
    let dhdx = cos(a) * 0.30 + cos(c) * 0.28 - sin(d) * 0.10;
    let dhdz = -sin(b) * 0.30 + cos(c) * 0.28 + sin(d) * 0.13;
    return vec2<f32>(dhdx, dhdz);
}

struct SheetHit {
    hit: bool;
    pos: vec3<f32>;
    normal: vec3<f32>;
    albedo: vec3<f32>;
    steps: u32;
};

fn sheet_at(x: f32, z: f32, steps: u32) -> SheetHit {
    let h = terrain_surface(x, z);
    let grad = terrain_gradient(x, z);
    let normal = normalize(vec3<f32>(-grad.x, 1.0, -grad.y));
    let slope = length(grad);
    let t = clamp((slope - 0.2) / 0.45, 0.0, 1.0);
    let cliff = t * t * (3.0 - 2.0 * t);
    let grass = vec3<f32>(0.337, 0.686, 0.196);
    let rock = vec3<f32>(0.34, 0.32, 0.30);
    let albedo = mix(grass, rock, cliff);
    let pos = blocks_to_octree(vec3<f32>(x, h, z));
    return SheetHit(true, pos, normal, albedo, steps);
}

fn heightfield_ray(start_octree: vec3<f32>, dir: vec3<f32>) -> SheetHit {
    var p = octree_to_blocks(start_octree);
    let horiz = length(vec2<f32>(dir.x, dir.z));
    let approach = TERRAIN_MAX_SLOPE * horiz - dir.y;
    if (approach <= 0.02) {
        let h = terrain_surface(p.x, p.z);
        if (p.y <= h) {
            return sheet_at(p.x, p.z, 0u);
        }
        return SheetHit(false, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), 0u);
    }

    // Crests cannot exceed the sum of the amplitudes, so the air above them is empty.
    if (p.y > TERRAIN_MAX_HEIGHT && dir.y < 0.0) {
        p = p + dir * ((p.y - TERRAIN_MAX_HEIGHT) / -dir.y);
    }

    var steps = 0u;
    loop {
        if (steps >= 96u) {
            break;
        }
        let h = terrain_surface(p.x, p.z);
        let g = p.y - h;
        if (g <= 0.05) {
            return sheet_at(p.x, p.z, steps);
        }
        // dir is a unit vector in both spaces: the cube is scaled uniformly.
        // The slope bound is above the real gradient, so this step meets the sheet.
        p = p + dir * (g / approach);
        steps = steps + 1u;
    }
    return SheetHit(false, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), steps);
}

fn sky_colour(dir: vec3<f32>) -> vec3<f32> {
    let t = clamp(dir.y * 0.45 + 0.35, 0.0, 1.0);
    return mix(vec3<f32>(0.66, 0.74, 0.80), vec3<f32>(0.32, 0.48, 0.74), t);
}

struct SceneHit {
    hit: bool;
    pos: vec3<f32>;
    // Face the ray entered. Offsets and "into the cube" tests use this.
    normal: vec3<f32>;
    // Stored voxel normal. Lighting uses this for every face of the voxel.
    shade: vec3<f32>;
    albedo: vec3<f32>;
    steps: u32;
    hit_count: f32;
    emission: f32;
    roughness: f32;
    metal: f32;
    // Octree node index. 0xffffffffu is a heightfield or miss, which has no record.
    leaf: u32;
};

fn expand5(v: u32) -> f32 {
    return f32((v << 3u) | (v >> 2u)) / 255.0;
}

fn expand6(v: u32) -> f32 {
    return f32((v << 2u) | (v >> 4u)) / 255.0;
}

fn material_roughness(q: u32) -> f32 {
    if (q == 0u) {
        return 0.72;
    }
    if (q == 1u) {
        return 0.28;
    }
    if (q == 2u) {
        return 0.62;
    }
    return 0.96;
}

// Inverse of pack_oct_normal. code 0 is not a direction. 1..=6 are the axes.
fn oct_normal(code: u32) -> vec3<f32> {
    if (code == 1u) { return vec3<f32>(1.0, 0.0, 0.0); }
    if (code == 2u) { return vec3<f32>(-1.0, 0.0, 0.0); }
    if (code == 3u) { return vec3<f32>(0.0, 1.0, 0.0); }
    if (code == 4u) { return vec3<f32>(0.0, -1.0, 0.0); }
    if (code == 5u) { return vec3<f32>(0.0, 0.0, 1.0); }
    if (code == 6u) { return vec3<f32>(0.0, 0.0, -1.0); }
    let idx = code - 7u;
    let u = f32(idx & 7u) / 7.0 * 2.0 - 1.0;
    let v = f32(idx >> 3u) / 7.0 * 2.0 - 1.0;
    var x = u;
    var y = v;
    var z = 1.0 - abs(u) - abs(v);
    if (z < 0.0) {
        let ox = x;
        let oy = y;
        var sx = 1.0;
        if (ox < 0.0) { sx = -1.0; }
        var sy = 1.0;
        if (oy < 0.0) { sy = -1.0; }
        x = (1.0 - abs(oy)) * sx;
        y = (1.0 - abs(ox)) * sy;
    }
    return normalize(vec3<f32>(x, y, z));
}

fn d_ggx(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (3.14159265 * d * d);
}

fn vis_smith(n_dot_v: f32, n_dot_l: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let gv = n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2);
    let gl = n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 0.0001);
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - cos_theta, 5.0);
}

fn basis(normal: vec3<f32>) -> vec3<f32> {
    var helper = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(normal.y) > 0.999) {
        helper = vec3<f32>(1.0, 0.0, 0.0);
    }
    return normalize(cross(helper, normal));
}

fn to_world(normal: vec3<f32>, local: vec3<f32>) -> vec3<f32> {
    let tangent = basis(normal);
    let bitangent = cross(normal, tangent);
    return normalize(tangent * local.x + normal * local.y + bitangent * local.z);
}

fn scene_ray(ray: Ray, primary: bool) -> SceneHit {
    let near = octree_ray(ray, primary);
    if (near.hit) {
        let value = node(near.value) - VOXEL_OFFSET;
        let albedo = vec3<f32>(
            expand5((value >> 11u) & 31u),
            expand6((value >> 5u) & 63u),
            expand5(value & 31u),
        );
        let hit_count = f32(n.data[near.value] & 15u) / 15.0;
        let emission_code = (value >> 16u) & 3u;
        var emission = 0.0;
        if (emission_code > 1u) {
            emission = f32(emission_code - 1u) / 2.0;
        }
        let normal_code = (value >> 21u) & 63u;
        var shade = near.normal;
        if (normal_code != 0u) {
            let mapped = oct_normal(normal_code);
            let bent = near.normal + (mapped - near.normal) * path.normal_scale;
            if (dot(bent, bent) > 0.0001) {
                shade = normalize(bent);
            }
        }
        return SceneHit(
            true,
            near.pos,
            near.normal,
            shade,
            albedo,
            near.steps,
            hit_count,
            emission,
            clamp(material_roughness((value >> 19u) & 3u) * path.roughness_scale, 0.04, 1.0),
            f32((value >> 18u) & 1u),
            near.value,
        );
    }

    if (terrain_enabled()) {
        let far = heightfield_ray(near.pos, ray.dir);
        if (far.hit) {
            return SceneHit(true, far.pos, far.normal, far.normal, far.albedo, near.steps + far.steps, 0.0, 0.0, 0.72, 0.0, 4294967295u);
        }
        return SceneHit(false, far.pos, far.normal, far.normal, far.albedo, near.steps + far.steps, 0.0, 0.0, 0.72, 0.0, 4294967295u);
    }

    return SceneHit(false, near.pos, near.normal, near.normal, vec3<f32>(0.0), near.steps, 0.0, 0.0, 1.0, 0.0, 4294967295u);
}

fn hash_u32(value: u32) -> u32 {
    var x = value;
    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);
    return x;
}

fn random_float(seed: u32) -> f32 {
    return f32(hash_u32(seed)) * (1.0 / 4294967296.0);
}

// Two blue-noise values for one hemisphere draw. The frame walks the tile so
// the next deposit lands in a gap instead of reclumping the same directions.
fn blue_disk(pixel: vec2<u32>, index: u32, slot: u32) -> vec2<f32> {
    let x = i32((pixel.x + index * 19u + path.frame_index + slot * 7u) % 64u);
    let y = i32((pixel.y + index * 13u + path.frame_index * 3u + slot) % 64u);
    return textureLoad(blue_noise, vec2<i32>(x, y), 0).rg;
}

fn cosine_from_blue(normal: vec3<f32>, sample_index: u32, sample_count: u32, rg: vec2<f32>) -> vec3<f32> {
    let width = f32(max(sample_count, 1u));
    let r2 = (f32(sample_index) + rg.x) / width;
    let phi = 6.28318530718 * rg.y;
    let radius = sqrt(r2);
    let local = vec3<f32>(cos(phi) * radius, sqrt(max(0.0, 1.0 - r2)), sin(phi) * radius);
    return to_world(normal, local);
}

fn sample_ggx(normal: vec3<f32>, roughness: f32, seed: u32) -> vec3<f32> {
    let alpha = roughness * roughness;
    let r1 = random_float(seed);
    let r2 = random_float(seed ^ 0x68bc21ebu);
    let phi = 6.28318530718 * r1;
    let cos_theta = sqrt((1.0 - r2) / (1.0 + (alpha * alpha - 1.0) * r2));
    let sin_theta = sqrt(max(0.0, 1.0 - cos_theta * cos_theta));
    return to_world(normal, vec3<f32>(cos(phi) * sin_theta, cos_theta, sin(phi) * sin_theta));
}

fn sun_visible(pos: vec3<f32>, normal: vec3<f32>, sun_direction: vec3<f32>) -> bool {
    if (!u.shadows) {
        return true;
    }
    let origin = pos + normal * 0.000004 + sun_direction * 0.000004;
    return !scene_ray(Ray(origin, sun_direction), false).hit;
}

struct PathSample {
    radiance: vec3<f32>;
    depth: f32;
    steps: u32;
    first_albedo: vec3<f32>;
    first_hit: bool;
    first_hit_count: f32;
    first_pos: vec3<f32>;
    first_normal: vec3<f32>;
};

fn luminance(colour: vec3<f32>) -> f32 {
    return dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The eight children of one parent sit on an 8-aligned index. A leaf borrows
// their light, weighted by how many samples each one holds. Its own record
// still stores only its own samples.
fn sibling_light(leaf: u32) -> vec4<f32> {
    let base = leaf & 0xfffffff8u;
    var acc = vec3<f32>(0.0);
    var weight = 0.0;
    var i = 0u;
    loop {
        if (i >= 8u) { break; }
        let index = base + i;
        let rec = voxel_light.data[index];
        if (rec.w > 0.0 && node(index) >= VOXEL_OFFSET) {
            acc = acc + rec.xyz * rec.w;
            weight = weight + rec.w;
        }
        i = i + 1u;
    }
    if (weight <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(acc / weight, 1.0);
}

// Stores this diffuse sample on the leaf. The pixel shows the mean from
// before this sample, so one noisy path does not repaint a settled voxel.
// The count stops growing at 64 and then each new ray replaces 1/64 of the
// mean, so walking into a new room keeps updating instead of freezing.
fn deposit_light(leaf: u32, sample_l: vec3<f32>) -> vec3<f32> {
    if (path.voxel_light == 0u) {
        return sample_l;
    }
    let rec = voxel_light.data[leaf];
    let n_before = u32(rec.w);
    if (n_before == 0u) {
        voxel_light.data[leaf] = vec4<f32>(sample_l, 1.0);
        let shared = sibling_light(leaf);
        if (shared.w > 0.0) {
            return shared.xyz;
        }
        return sample_l;
    }
    var n = n_before;
    var mean = rec.xyz;
    if (n > 64u) {
        n = 64u;
    }
    if (n < 64u) {
        n = n + 1u;
        mean = mean + (sample_l - mean) / f32(n);
    } else {
        mean = mean + (sample_l - mean) * (1.0 / 64.0);
    }
    voxel_light.data[leaf] = vec4<f32>(mean, f32(n));
    let shared = sibling_light(leaf);
    if (shared.w > 0.0) {
        return shared.xyz;
    }
    return rec.xyz;
}

// One bounce, read only. The four probes that feed a deposit must not write,
// or they would race each other on the next leaf.
fn sample_bounce(ray: Ray) -> vec3<f32> {
    let hit = scene_ray(ray, false);
    if (!hit.hit) {
        return sky_colour(ray.dir) * 1.5;
    }
    let diffuse_albedo = hit.albedo * (1.0 - hit.metal);
    let emitted = hit.albedo * (hit.emission * path.texture_emission);
    if (hit.leaf != 4294967295u) {
        let shared = sibling_light(hit.leaf);
        if (shared.w > 0.0) {
            return emitted + diffuse_albedo * shared.xyz;
        }
    }
    let sun_direction = normalize(-u.sun_dir.xyz);
    let n_dot_l = max(dot(hit.shade, sun_direction), 0.0);
    var direct_l = vec3<f32>(0.0);
    if (n_dot_l > 0.0 && sun_visible(hit.pos, hit.normal, sun_direction)) {
        direct_l = vec3<f32>(2.4, 2.25, 2.0) * n_dot_l;
    }
    return emitted + diffuse_albedo * direct_l;
}

// The bounce budget ends by reading the next surface instead of dropping that
// ray. A miss is sky. A hit is the light that voxel has already accumulated,
// which is how a later bounce shows up without retracing it.
fn terminal_radiance(ray: Ray) -> vec3<f32> {
    let hit = scene_ray(ray, true);
    if (!hit.hit) {
        return sky_colour(ray.dir) * 1.5;
    }
    let diffuse_albedo = hit.albedo * (1.0 - hit.metal);
    let emitted = hit.albedo * (hit.emission * path.texture_emission);
    if (hit.leaf != 4294967295u) {
        let shared = sibling_light(hit.leaf);
        if (shared.w > 0.0) {
            return emitted + diffuse_albedo * shared.xyz;
        }
    }
    let sun_direction = normalize(-u.sun_dir.xyz);
    let n_dot_l = max(dot(hit.shade, sun_direction), 0.0);
    var direct_l = vec3<f32>(0.0);
    if (n_dot_l > 0.0 && sun_visible(hit.pos, hit.normal, sun_direction)) {
        direct_l = vec3<f32>(2.4, 2.25, 2.0) * n_dot_l;
    }
    if (hit.leaf != 4294967295u && luminance(diffuse_albedo) > 0.001) {
        deposit_light(hit.leaf, direct_l);
    }
    return emitted + diffuse_albedo * direct_l;
}

fn trace_path(camera_pos: vec3<f32>, camera_dir: vec3<f32>, seed: u32, gather: bool, pixel: vec2<u32>) -> PathSample {
    var ray = Ray(camera_pos, camera_dir);
    // Kept only so Russian roulette sees the same throughput as before.
    var throughput = vec3<f32>(1.0);
    var depth = 1000.0;
    var first_steps = 0u;
    var first_albedo = vec3<f32>(0.0);
    var first_hit = false;
    var first_hit_count = 0.0;
    var first_pos = vec3<f32>(0.0);
    var first_normal = vec3<f32>(0.0);
    var incoming = vec3<f32>(0.0);
    var count = 0u;
    var bounce = 0u;

    loop {
        // max_bounces is the number of bounces after the camera hit. 1 traces
        // the wall you see and the surface the bounce lands on, and both store light.
        if (bounce > path.max_bounces || bounce >= 6u) { break; }
        // Every bounce may split the leaf it hits. Stopping after the camera
        // ray keeps the detail on the screen and the light follows it.
        let hit = scene_ray(ray, true);
        if (bounce == 0u) {
            first_steps = hit.steps;
            first_albedo = hit.albedo;
            first_hit = hit.hit;
            first_hit_count = hit.hit_count;
            if (hit.hit) {
                depth = length(hit.pos - camera_pos);
                first_pos = hit.pos;
                first_normal = hit.shade;
            }
        }

        if (!hit.hit) {
            incoming = sky_colour(ray.dir) * 1.5;
            break;
        }

        let shade_n = hit.shade;
        let view = -ray.dir;
        let alpha = hit.roughness * hit.roughness;
        let f0 = mix(vec3<f32>(0.04), hit.albedo, hit.metal);
        let diffuse_albedo = hit.albedo * (1.0 - hit.metal);
        let spec_share = luminance(f0);
        let diffuse_share = luminance(diffuse_albedo);
        let p_spec = clamp(spec_share / max(spec_share + diffuse_share, 0.001), 0.0, 0.85);
        let emitted = hit.albedo * (hit.emission * path.texture_emission);

        let sun_direction = normalize(-u.sun_dir.xyz);
        let n_dot_l = max(dot(shade_n, sun_direction), 0.0);
        let n_dot_v = max(dot(shade_n, view), 0.001);
        let sun_rgb = vec3<f32>(2.4, 2.25, 2.0);
        var direct_l = vec3<f32>(0.0);
        var spec_emit = emitted;
        if (n_dot_l > 0.0 && sun_visible(hit.pos, hit.normal, sun_direction)) {
            let half_sun = normalize(view + sun_direction);
            let n_dot_h = max(dot(shade_n, half_sun), 0.0);
            let fresnel = fresnel_schlick(max(dot(view, half_sun), 0.0), f0);
            let spec = d_ggx(n_dot_h, alpha) * vis_smith(n_dot_v, n_dot_l, alpha) * fresnel;
            let sun_radiance = sun_rgb * 3.14159265;
            let diffuse_sun = diffuse_albedo * sun_rgb * n_dot_l;
            let spec_sun = spec * sun_radiance * n_dot_l;
            let sun_sum = diffuse_sun + spec_sun;
            var sun_scale = 1.0;
            let peak = max(max(sun_sum.x, sun_sum.y), sun_sum.z);
            if (peak > 12.0) {
                sun_scale = 12.0 / peak;
            }
            direct_l = sun_rgb * n_dot_l * sun_scale;
            spec_emit = emitted + spec_sun * sun_scale;
        }

        light_leaf[bounce] = hit.leaf;
        light_has[bounce] = u32(hit.leaf != 4294967295u);
        light_direct[bounce] = direct_l;
        light_emit[bounce] = spec_emit;
        light_albedo[bounce] = diffuse_albedo;
        light_added[bounce] = vec3<f32>(0.0);

        // Lighting scale 2 keeps one pixel of each 2x2, scale 3 one of each 3x3.
        // The other pixels still find the surface and read the stored light.
        if (!gather) {
            light_spec[bounce] = vec3<f32>(0.0);
            light_indirect[bounce] = 0.0;
            count = bounce + 1u;
            break;
        }

        let bounce_seed = hash_u32(seed ^ (bounce + 1u) * 0x9e3779b9u);
        let spec_bounce = p_spec > 0.001 && random_float(bounce_seed) < p_spec;
        var next_dir = vec3<f32>(0.0);
        var spec_w = vec3<f32>(0.0);
        var indirect_scale = 0.0;
        if (spec_bounce) {
            let half_vec = sample_ggx(shade_n, hit.roughness, bounce_seed ^ 0x632be59bu);
            next_dir = reflect(-view, half_vec);
            let spec_ndl = max(dot(shade_n, next_dir), 0.0);
            let spec_ndh = max(dot(shade_n, half_vec), 0.001);
            let spec_vdh = max(dot(view, half_vec), 0.001);
            if (spec_ndl <= 0.0 || dot(next_dir, hit.normal) <= 0.05) {
                light_spec[bounce] = vec3<f32>(0.0);
                light_indirect[bounce] = 0.0;
                count = bounce + 1u;
                break;
            }
            let fresnel = fresnel_schlick(spec_vdh, f0);
            let vis = vis_smith(n_dot_v, spec_ndl, alpha);
            let weight = vis * fresnel * spec_ndl * 4.0 * spec_vdh / spec_ndh;
            spec_w = clamp(weight, vec3<f32>(0.0), vec3<f32>(4.0)) / p_spec;
            throughput = throughput * spec_w;
        } else {
            next_dir = cosine_from_blue(shade_n, bounce, 1u, blue_disk(pixel, bounce, 0u));
            if (dot(next_dir, hit.normal) <= 0.05) {
                next_dir = cosine_from_blue(shade_n, bounce, 1u, blue_disk(pixel, bounce, 3u));
            }
            indirect_scale = 1.0 / max(1.0 - p_spec, 0.001);
            throughput = throughput * diffuse_albedo * indirect_scale;
            // The extra rays would be thrown away if each one stored itself,
            // so they are averaged in the pixel and written once.
            if (path.voxel_light != 0u) {
                let samples = max(path.light_samples, 1u);
                var probe_sum = vec3<f32>(0.0);
                var s = 0u;
                loop {
                    if (s >= samples) { break; }
                    let rg = blue_disk(pixel, bounce * 8u + s, 1u);
                    var probe_dir = cosine_from_blue(shade_n, s, samples, rg);
                    if (dot(probe_dir, hit.normal) <= 0.05) {
                        probe_dir = cosine_from_blue(shade_n, s, samples, blue_disk(pixel, bounce * 8u + s, 4u));
                    }
                    let origin = hit.pos + hit.normal * 0.000004 + probe_dir * 0.000004;
                    probe_sum = probe_sum + sample_bounce(Ray(origin, probe_dir));
                    s = s + 1u;
                }
                light_added[bounce] = probe_sum * (indirect_scale / f32(samples));
            }
        }
        if (bounce >= 2u) {
            let survive = clamp(max(max(throughput.x, throughput.y), throughput.z), 0.1, 0.95);
            if (random_float(seed ^ hash_u32(bounce * 0x632be59bu)) > survive) {
                light_spec[bounce] = vec3<f32>(0.0);
                light_indirect[bounce] = 0.0;
                count = bounce + 1u;
                break;
            }
            spec_w = spec_w / survive;
            indirect_scale = indirect_scale / survive;
            throughput = throughput / survive;
        }
        light_spec[bounce] = spec_w;
        light_indirect[bounce] = indirect_scale;
        count = bounce + 1u;
        ray = Ray(hit.pos + hit.normal * 0.000004 + next_dir * 0.000004, next_dir);
        bounce = bounce + 1u;
    }

    // The loop stops with one ray still in hand. Resolve it from the voxel
    // cache or the sky so that bounce is stored on the surface it left.
    if (path.voxel_light == 0u
        && count > 0u
        && light_indirect[count - 1u] > 0.0
        && incoming.x == 0.0
        && incoming.y == 0.0
        && incoming.z == 0.0)
    {
        incoming = terminal_radiance(ray);
    }

    // Walk back to the camera. Each diffuse hit stores its sample and then
    // shades with the mean, so later pixels and later views share the light.
    var li = incoming;
    loop {
        if (count == 0u) { break; }
        count = count - 1u;
        var lit = light_direct[count];
        if (path.voxel_light != 0u) {
            lit = lit + light_added[count];
        } else if (light_indirect[count] > 0.0) {
            lit = lit + li * light_indirect[count];
        }
        var diffuse_out = light_albedo[count] * lit;
        if (light_has[count] != 0u && luminance(light_albedo[count]) > 0.001) {
            if (gather) {
                diffuse_out = light_albedo[count] * deposit_light(light_leaf[count], lit);
            } else {
                let leaf = light_leaf[count];
                let shared = sibling_light(leaf);
                if (shared.w > 0.0) {
                    diffuse_out = light_albedo[count] * shared.xyz;
                }
            }
        }
        // The random specular bounce is only here to carry light onto other
        // voxels. The pixel keeps the direct highlight, which does not flicker.
        var continuation = light_spec[count] * li;
        if (path.voxel_light != 0u) {
            continuation = vec3<f32>(0.0);
        }
        li = light_emit[count] + diffuse_out + continuation;
    }

    return PathSample(
        li,
        depth,
        first_steps,
        first_albedo,
        first_hit,
        first_hit_count,
        first_pos,
        first_normal,
    );
}

fn history_matches(previous: HistoryPixel, pos: vec3<f32>, normal: vec3<f32>, depth: f32) -> bool {
    let position_tolerance = 0.003 + depth * 0.002;
    return previous.position.w > 0.5
        && distance(previous.position.xyz, pos) < position_tolerance
        && dot(previous.normal.xyz, normal) > 0.8;
}

struct TemporalHistory {
    illumination: vec3<f32>;
    moment1: f32;
    moment2: f32;
    history_length: f32;
    weight: f32;
};

fn consider_history(
    coord: vec2<i32>,
    weight: f32,
    pos: vec3<f32>,
    normal: vec3<f32>,
    depth: f32,
    acc: TemporalHistory,
) -> TemporalHistory {
    if (weight <= 0.0 || coord.x < 0 || coord.y < 0) {
        return acc;
    }
    if (coord.x >= i32(u.dimensions.x) || coord.y >= i32(u.dimensions.y)) {
        return acc;
    }
    let index = u32(coord.y) * u32(u.dimensions.x) + u32(coord.x);
    let previous = history_in.data[index];
    if (!history_matches(previous, pos, normal, depth)) {
        return acc;
    }
    var next: TemporalHistory;
    // radiance.xyz is last frame's filtered illumination. Moments stay
    // unfiltered so the variance estimate does not collapse across frames.
    next.illumination = acc.illumination + previous.radiance.xyz * weight;
    next.moment1 = acc.moment1 + previous.moments.x * weight;
    next.moment2 = acc.moment2 + previous.moments.y * weight;
    next.history_length = acc.history_length + previous.radiance.w * weight;
    next.weight = acc.weight + weight;
    return next;
}

struct FSOut {
    [[location(0)]] illumination: vec4<f32>;
    [[location(1)]] normal: vec4<f32>;
    [[location(2)]] albedo: vec4<f32>;
    [[location(3)]] moments: vec4<f32>;
    [[builtin(frag_depth)]] depth: f32;
};

[[stage(fragment)]]
fn fs_main(in: FSIn) -> FSOut {
    let pixel = vec2<u32>(in.frag_pos.xy);
    let pixel_index = pixel.y * u32(u.dimensions.x) + pixel.x;
    let noise_coord = vec2<i32>(i32(pixel.x % 64u), i32(pixel.y % 64u));
    let blue = textureLoad(blue_noise, noise_coord, 0).rg;
    let frame_shift = vec2<f32>(
        random_float(hash_u32(path.frame_index * 0x9e3779b9u)),
        random_float(hash_u32(path.frame_index * 0x68bc21ebu + 1u)),
    );
    let jitter = fract(blue + frame_shift) - vec2<f32>(0.5);
    let blue_seed = u32(blue.x * 255.0) | (u32(blue.y * 255.0) << 8u);
    let seed = hash_u32(pixel.x + pixel.y * 65537u + path.frame_index * 0x9e3779b9u ^ blue_seed);
    let clip_space = get_clip_space(in.frag_pos + vec4<f32>(jitter, 0.0, 0.0), u.dimensions.xy);

    let pos = u.camera_inverse * vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let dir = u.camera_inverse * vec4<f32>(clip_space.x, clip_space.y, 1.0, 1.0);
    let pos = pos.xyz / pos.w;
    let dir = normalize(dir.xyz / dir.w - pos);
    // 0 is a 2×2 lighting grid inside the pixel. 2 and 3 keep one pixel per cell.
    let scale = path.light_scale;
    var gather = true;
    if (path.voxel_light != 0u && scale > 1u) {
        let cell = scale * scale;
        let phase = path.frame_index % cell;
        gather = (pixel.x % scale) == (phase % scale) && (pixel.y % scale) == (phase / scale);
    }
    let fine = path.voxel_light != 0u && scale == 0u && !u.show_steps && !u.show_hits;
    var ray_dir = dir;
    if (fine) {
        let sub_clip = get_clip_space(in.frag_pos + vec4<f32>(jitter.x - 0.25, jitter.y - 0.25, 0.0, 0.0), u.dimensions.xy);
        let sub_dir_h = u.camera_inverse * vec4<f32>(sub_clip.x, sub_clip.y, 1.0, 1.0);
        ray_dir = normalize(sub_dir_h.xyz / sub_dir_h.w - pos);
    }
    var sample = trace_path(pos, ray_dir, seed, gather, pixel);
    if (fine) {
        var sum = sample.radiance;
        var s = 1u;
        loop {
            if (s >= 4u) { break; }
            var ox = -0.25;
            if ((s & 1u) == 1u) { ox = 0.25; }
            var oy = -0.25;
            if (s >= 2u) { oy = 0.25; }
            let sub_clip = get_clip_space(in.frag_pos + vec4<f32>(jitter.x + ox, jitter.y + oy, 0.0, 0.0), u.dimensions.xy);
            let sub_dir_h = u.camera_inverse * vec4<f32>(sub_clip.x, sub_clip.y, 1.0, 1.0);
            let sub_dir = normalize(sub_dir_h.xyz / sub_dir_h.w - pos);
            let sub_seed = hash_u32(seed ^ (s * 0x85ebca6bu));
            sum = sum + trace_path(pos, sub_dir, sub_seed, true, pixel).radiance;
            s = s + 1u;
        }
        sample.radiance = sum * 0.25;
    }
    var sample_colour = sample.radiance;

    if (u.show_steps) {
        sample_colour = vec3<f32>(f32(sample.steps) / 64.0);
    } else if (u.show_hits && sample.first_hit) {
        sample_colour = vec3<f32>(sample.first_hit_count);
    }

    // SVGF filters illumination, then multiplies the center albedo back on.
    // Dividing by the first-hit albedo keeps the wavelet from smearing textures.
    var albedo = vec3<f32>(1.0);
    if (sample.first_hit && !u.show_steps && !u.show_hits) {
        // Floor keeps demodulation invertible when a texel channel is black.
        albedo = max(sample.first_albedo, vec3<f32>(0.001));
    }
    let noisy_illumination = sample_colour / albedo;
    let noisy_luma = luminance(noisy_illumination);
    var integrated = noisy_illumination;
    var moment1 = noisy_luma;
    var moment2 = noisy_luma * noisy_luma;
    var history_length = 1.0;

    if (path.accumulate != 0u) {
        // The camera has not moved, so this pixel's earlier samples are the same
        // ray. Average them all. Temporal reprojection is only for when it moves.
        let count = max(f32(path.still_frames), 1.0);
        let alpha = 1.0 / count;
        let previous = history_in.data[pixel_index];
        integrated = mix(previous.radiance.xyz, noisy_illumination, alpha);
        moment1 = mix(previous.moments.x, noisy_luma, alpha);
        moment2 = mix(previous.moments.y, noisy_luma * noisy_luma, alpha);
        history_length = count;
    } else if (path.history_valid != 0u && sample.first_hit && path.temporal_history > 0.0) {
        let previous_clip = path.previous_camera * vec4<f32>(sample.first_pos, 1.0);
        // create_proj_matrix is the renderer's inverse-ray basis rather than a
        // conventional homogeneous projection. Its transformed z is the
        // perspective divisor; w remains 1 for world positions.
        if (previous_clip.z > 0.00001) {
            let previous_ndc = previous_clip.xy / previous_clip.z;
            let previous_uv = previous_ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
            if (all(previous_uv >= vec2<f32>(0.0)) && all(previous_uv < vec2<f32>(1.0))) {
                // Bilinear footprints sit halfway between pixel centers.
                let tex = previous_uv * u.dimensions.xy - vec2<f32>(0.5);
                let origin = vec2<i32>(floor(tex));
                let frac = fract(tex);
                var acc: TemporalHistory;
                acc.illumination = vec3<f32>(0.0);
                acc.moment1 = 0.0;
                acc.moment2 = 0.0;
                acc.history_length = 0.0;
                acc.weight = 0.0;
                acc = consider_history(origin + vec2<i32>(0, 0), (1.0 - frac.x) * (1.0 - frac.y), sample.first_pos, sample.first_normal, sample.depth, acc);
                acc = consider_history(origin + vec2<i32>(1, 0), frac.x * (1.0 - frac.y), sample.first_pos, sample.first_normal, sample.depth, acc);
                acc = consider_history(origin + vec2<i32>(0, 1), (1.0 - frac.x) * frac.y, sample.first_pos, sample.first_normal, sample.depth, acc);
                acc = consider_history(origin + vec2<i32>(1, 1), frac.x * frac.y, sample.first_pos, sample.first_normal, sample.depth, acc);
                if (acc.weight > 0.0) {
                    let inv = 1.0 / acc.weight;
                    history_length = min(acc.history_length * inv + 1.0, 64.0);
                    // The slider is the steady-state history weight. Early frames
                    // still use 1/N so the moments are an unbiased running estimate.
                    let alpha = max(1.0 - path.temporal_history, 1.0 / history_length);
                    integrated = mix(acc.illumination * inv, noisy_illumination, alpha);
                    moment1 = mix(acc.moment1 * inv, noisy_luma, alpha);
                    moment2 = mix(acc.moment2 * inv, noisy_luma * noisy_luma, alpha);
                }
            }
        }
    }

    history_out.data[pixel_index].radiance = vec4<f32>(integrated, history_length);
    history_out.data[pixel_index].position = vec4<f32>(sample.first_pos, f32(sample.first_hit));
    history_out.data[pixel_index].normal = vec4<f32>(sample.first_normal, sample.depth);
    history_out.data[pixel_index].moments = vec4<f32>(moment1, moment2, 0.0, 0.0);
    history_out.data[pixel_index].albedo = vec4<f32>(albedo, 0.0);

    let depth = sample.depth / (sample.depth + 1.0);
    return FSOut(
        vec4<f32>(integrated, history_length),
        vec4<f32>(sample.first_normal, sample.depth),
        vec4<f32>(albedo, f32(sample.first_hit)),
        vec4<f32>(moment1, moment2, 0.0, 0.0),
        depth,
    );
}
