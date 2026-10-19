struct VSOut {
    [[builtin(position)]] clip: vec4<f32>;
};

struct TaaUniforms {
    camera_inverse: mat4x4<f32>;
    previous_camera: mat4x4<f32>;
    dimensions: vec2<f32>;
    history_valid: u32;
    frame_index: u32;
};

[[group(0), binding(0)]]
var current_color: texture_2d<f32>;
[[group(0), binding(1)]]
var current_depth: texture_depth_2d;
[[group(0), binding(2)]]
var history_color: texture_2d<f32>;
[[group(0), binding(3)]]
var<uniform> u: TaaUniforms;

[[stage(vertex)]]
fn vs_main([[builtin(vertex_index)]] vertex: u32) -> VSOut {
    var out: VSOut;
    if (vertex == 0u) {
        out.clip = vec4<f32>(-1.0, -1.0, 0.0, 1.0);
    } else if (vertex == 1u) {
        out.clip = vec4<f32>(1.0, -1.0, 0.0, 1.0);
    } else if (vertex == 2u) {
        out.clip = vec4<f32>(-1.0, 1.0, 0.0, 1.0);
    } else {
        out.clip = vec4<f32>(1.0, 1.0, 0.0, 1.0);
    }
    return out;
}

fn halton(index: u32, base: u32) -> f32 {
    var i = index;
    var f = 1.0;
    var r = 0.0;
    loop {
        if (i == 0u) { break; }
        f = f / f32(base);
        r = r + f * f32(i % base);
        i = i / base;
    }
    return r;
}

// Same 16-sample sequence as the ray and the mesh pass, in pixels.
fn taa_jitter(frame: u32) -> vec2<f32> {
    let i = (frame % 16u) + 1u;
    return vec2<f32>(halton(i, 2u), halton(i, 3u)) - vec2<f32>(0.5);
}

fn get_clip_space(frag_pos: vec2<f32>, dimensions: vec2<f32>) -> vec2<f32> {
    var clip_space = frag_pos / dimensions * 2.0;
    clip_space = clip_space - 1.0;
    clip_space = clip_space * vec2<f32>(1.0, -1.0);
    return clip_space;
}

fn load_current(p: vec2<i32>, dims: vec2<i32>) -> vec3<f32> {
    let coord = clamp(p, vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(current_color, coord, 0).rgb;
}

fn load_history(p: vec2<i32>, dims: vec2<i32>) -> vec3<f32> {
    let coord = clamp(p, vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(history_color, coord, 0).rgb;
}

[[stage(fragment)]]
fn fs_main(in: VSOut) -> [[location(0)]] vec4<f32> {
    let dims = vec2<i32>(u.dimensions);
    let coord = vec2<i32>(in.clip.xy);
    let current = load_current(coord, dims);
    if (u.history_valid == 0u) {
        return vec4<f32>(current, 1.0);
    }
    let d = textureLoad(current_depth, coord, 0);
    if (d <= 0.0001 || d >= 0.999) {
        return vec4<f32>(current, 1.0);
    }
    let dist = d / (1.0 - d);
    let jitter = taa_jitter(u.frame_index);
    let clip_space = get_clip_space(in.clip.xy + jitter, u.dimensions);
    let origin_h = u.camera_inverse * vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let dir_h = u.camera_inverse * vec4<f32>(clip_space.x, clip_space.y, 1.0, 1.0);
    let origin = origin_h.xyz / origin_h.w;
    let dir = normalize(dir_h.xyz / dir_h.w - origin);
    let world = origin + dir * dist;
    let previous_clip = u.previous_camera * vec4<f32>(world, 1.0);
    if (previous_clip.z <= 0.00001) {
        return vec4<f32>(current, 1.0);
    }
    let previous_ndc = previous_clip.xy / previous_clip.z;
    let previous_uv = previous_ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if (any(previous_uv < vec2<f32>(0.0)) || any(previous_uv >= vec2<f32>(1.0))) {
        return vec4<f32>(current, 1.0);
    }
    let tex = previous_uv * u.dimensions - vec2<f32>(0.5);
    let origin_px = vec2<i32>(floor(tex));
    let frac = fract(tex);
    let h00 = load_history(origin_px, dims);
    let h10 = load_history(origin_px + vec2<i32>(1, 0), dims);
    let h01 = load_history(origin_px + vec2<i32>(0, 1), dims);
    let h11 = load_history(origin_px + vec2<i32>(1, 1), dims);
    let history = mix(mix(h00, h10, frac.x), mix(h01, h11, frac.x), frac.y);

    // Clip history to this frame's neighbourhood so a disoccluded edge
    // cannot drag the old colour across the silhouette.
    var cmin = current;
    var cmax = current;
    let n0 = load_current(coord + vec2<i32>(-1, -1), dims);
    let n1 = load_current(coord + vec2<i32>(0, -1), dims);
    let n2 = load_current(coord + vec2<i32>(1, -1), dims);
    let n3 = load_current(coord + vec2<i32>(-1, 0), dims);
    let n4 = load_current(coord + vec2<i32>(1, 0), dims);
    let n5 = load_current(coord + vec2<i32>(-1, 1), dims);
    let n6 = load_current(coord + vec2<i32>(0, 1), dims);
    let n7 = load_current(coord + vec2<i32>(1, 1), dims);
    cmin = min(cmin, min(n0, min(n1, n2)));
    cmin = min(cmin, min(n3, min(n4, n5)));
    cmin = min(cmin, min(n6, n7));
    cmax = max(cmax, max(n0, max(n1, n2)));
    cmax = max(cmax, max(n3, max(n4, n5)));
    cmax = max(cmax, max(n6, n7));
    let clipped = clamp(history, cmin, cmax);
    let resolved = mix(clipped, current, 0.1);
    return vec4<f32>(resolved, 1.0);
}
