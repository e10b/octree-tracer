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
    padding0: u32;
    padding1: u32;
    padding2: u32;
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
    normal: vec3<f32>;
    albedo: vec3<f32>;
    steps: u32;
    hit_count: f32;
    emission: f32;
};

fn scene_ray(ray: Ray, primary: bool) -> SceneHit {
    let near = octree_ray(ray, primary);
    if (near.hit) {
        let value = node(near.value) - VOXEL_OFFSET;
        let albedo = vec3<f32>(unpack_u8(value).yzw) / 255.0;
        let hit_count = f32(n.data[near.value] & 15u) / 15.0;
        let emission_code = (value >> 24u) & 7u;
        var emission = 0.0;
        if (emission_code > 1u) {
            emission = f32(emission_code - 1u) / 6.0;
        }
        return SceneHit(
            true,
            near.pos,
            near.normal,
            albedo,
            near.steps,
            hit_count,
            emission,
        );
    }

    if (terrain_enabled()) {
        let far = heightfield_ray(near.pos, ray.dir);
        if (far.hit) {
            return SceneHit(true, far.pos, far.normal, far.albedo, near.steps + far.steps, 0.0, 0.0);
        }
        return SceneHit(false, far.pos, far.normal, far.albedo, near.steps + far.steps, 0.0, 0.0);
    }

    return SceneHit(false, near.pos, near.normal, vec3<f32>(0.0), near.steps, 0.0, 0.0);
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

fn cosine_hemisphere(normal: vec3<f32>, seed: u32) -> vec3<f32> {
    let r1 = random_float(seed);
    let r2 = random_float(seed ^ 0x9e3779b9u);
    let phi = 6.28318530718 * r1;
    let radius = sqrt(r2);
    let local = vec3<f32>(cos(phi) * radius, sqrt(1.0 - r2), sin(phi) * radius);

    var helper = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(normal.y) > 0.999) {
        helper = vec3<f32>(1.0, 0.0, 0.0);
    }
    let tangent = normalize(cross(helper, normal));
    let bitangent = cross(normal, tangent);
    return normalize(tangent * local.x + normal * local.y + bitangent * local.z);
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

fn trace_path(camera_pos: vec3<f32>, camera_dir: vec3<f32>, seed: u32) -> PathSample {
    var ray = Ray(camera_pos, camera_dir);
    var throughput = vec3<f32>(1.0);
    var radiance = vec3<f32>(0.0);
    var depth = 1000.0;
    var first_steps = 0u;
    var first_albedo = vec3<f32>(0.0);
    var first_hit = false;
    var first_hit_count = 0.0;
    var first_pos = vec3<f32>(0.0);
    var first_normal = vec3<f32>(0.0);
    var bounce = 0u;

    loop {
        if (bounce >= path.max_bounces) { break; }
        // The first indirect segment must refine geometry too. Otherwise portals
        // outside the camera frustum remain represented by opaque coarse voxels,
        // so bounced skylight can never discover them.
        let hit = scene_ray(ray, bounce <= 1u);
        if (bounce == 0u) {
            first_steps = hit.steps;
            first_albedo = hit.albedo;
            first_hit = hit.hit;
            first_hit_count = hit.hit_count;
            if (hit.hit) {
                depth = length(hit.pos - camera_pos);
                first_pos = hit.pos;
                first_normal = hit.normal;
            }
        }

        if (!hit.hit) {
            radiance = radiance + throughput * sky_colour(ray.dir) * 1.5;
            break;
        }

        let emitted = hit.albedo * (hit.emission * path.texture_emission);
        radiance = radiance + throughput * emitted;

        let sun_direction = normalize(-u.sun_dir.xyz);
        let n_dot_l = max(dot(hit.normal, sun_direction), 0.0);
        if (n_dot_l > 0.0 && sun_visible(hit.pos, hit.normal, sun_direction)) {
            let sun_radiance = vec3<f32>(2.4, 2.25, 2.0);
            radiance = radiance + throughput * hit.albedo * sun_radiance * n_dot_l;
        }

        throughput = throughput * hit.albedo;
        if (bounce >= 2u) {
            let survive = clamp(max(max(throughput.x, throughput.y), throughput.z), 0.1, 0.95);
            if (random_float(seed ^ hash_u32(bounce * 0x632be59bu)) > survive) {
                break;
            }
            throughput = throughput / survive;
        }

        let bounce_seed = hash_u32(seed ^ (bounce + 1u) * 0x9e3779b9u);
        let next_dir = cosine_hemisphere(hit.normal, bounce_seed);
        ray = Ray(hit.pos + hit.normal * 0.000004 + next_dir * 0.000004, next_dir);
        bounce = bounce + 1u;
    }

    return PathSample(
        radiance,
        depth,
        first_steps,
        first_albedo,
        first_hit,
        first_hit_count,
        first_pos,
        first_normal,
    );
}

fn luminance(colour: vec3<f32>) -> f32 {
    return dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
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
    let sample = trace_path(pos, dir, seed);
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

    if (path.history_valid != 0u && sample.first_hit) {
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
