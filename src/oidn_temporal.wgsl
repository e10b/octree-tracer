// Blends this frame's OIDN output with the previous denoised frame.
// Reprojection and the accept test match the path tracer's temporal branch:
// the hit must land on the same world position and shading normal.

struct VSOut {
    [[builtin(position)]] clip: vec4<f32>;
};

struct HistoryPixel {
    radiance: vec4<f32>;
    position: vec4<f32>;
    normal: vec4<f32>;
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
    voxel_light: u32;
    light_scale: u32;
    light_samples: u32;
    normal_scale: f32;
    roughness_scale: f32;
    previous_camera: mat4x4<f32>;
};

struct BlendUniforms {
    ready: u32;
    width: u32;
    height: u32;
    pad: u32;
};

struct HistoryColor {
    color: vec3<f32>;
    weight: f32;
};

[[group(0), binding(0)]]
var current_color: texture_2d<f32>;
[[group(0), binding(1)]]
var history_color: texture_2d<f32>;
[[group(0), binding(2)]]
var<storage, read_write> history_prev: History;
[[group(0), binding(3)]]
var<storage, read_write> history_curr: History;
[[group(0), binding(4)]]
var<uniform> path: PathUniforms;
[[group(0), binding(5)]]
var<uniform> blend: BlendUniforms;

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

fn consider_denoised(
    coord: vec2<i32>,
    weight: f32,
    pos: vec3<f32>,
    normal: vec3<f32>,
    depth: f32,
    acc: HistoryColor,
) -> HistoryColor {
    if (weight <= 0.0 || coord.x < 0 || coord.y < 0) {
        return acc;
    }
    if (coord.x >= i32(blend.width) || coord.y >= i32(blend.height)) {
        return acc;
    }
    let index = u32(coord.y) * blend.width + u32(coord.x);
    let previous = history_prev.data[index];
    let position_tolerance = 0.003 + depth * 0.002;
    if (previous.position.w <= 0.5) {
        return acc;
    }
    if (distance(previous.position.xyz, pos) >= position_tolerance) {
        return acc;
    }
    if (dot(previous.normal.xyz, normal) <= 0.8) {
        return acc;
    }
    var next: HistoryColor;
    next.color = acc.color + textureLoad(history_color, coord, 0).rgb * weight;
    next.weight = acc.weight + weight;
    return next;
}

[[stage(fragment)]]
fn fs_main(in: VSOut) -> [[location(0)]] vec4<f32> {
    let pixel = vec2<i32>(in.clip.xy);
    let current = textureLoad(current_color, pixel, 0).rgb;
    if (blend.ready == 0u || path.history_valid == 0u || path.temporal_history <= 0.0) {
        return vec4<f32>(current, 1.0);
    }
    let index = u32(pixel.y) * blend.width + u32(pixel.x);
    let here = history_curr.data[index];
    if (here.position.w <= 0.5) {
        return vec4<f32>(current, 1.0);
    }
    let previous_clip = path.previous_camera * vec4<f32>(here.position.xyz, 1.0);
    if (previous_clip.z <= 0.00001) {
        return vec4<f32>(current, 1.0);
    }
    let previous_ndc = previous_clip.xy / previous_clip.z;
    let previous_uv = previous_ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let outside = previous_uv.x < 0.0
        || previous_uv.y < 0.0
        || previous_uv.x >= 1.0
        || previous_uv.y >= 1.0;
    if (outside) {
        return vec4<f32>(current, 1.0);
    }
    let tex = previous_uv * vec2<f32>(f32(blend.width), f32(blend.height)) - vec2<f32>(0.5);
    let origin = vec2<i32>(floor(tex));
    let frac = fract(tex);
    var acc: HistoryColor;
    acc.color = vec3<f32>(0.0);
    acc.weight = 0.0;
    acc = consider_denoised(origin + vec2<i32>(0, 0), (1.0 - frac.x) * (1.0 - frac.y), here.position.xyz, here.normal.xyz, here.normal.w, acc);
    acc = consider_denoised(origin + vec2<i32>(1, 0), frac.x * (1.0 - frac.y), here.position.xyz, here.normal.xyz, here.normal.w, acc);
    acc = consider_denoised(origin + vec2<i32>(0, 1), (1.0 - frac.x) * frac.y, here.position.xyz, here.normal.xyz, here.normal.w, acc);
    acc = consider_denoised(origin + vec2<i32>(1, 1), frac.x * frac.y, here.position.xyz, here.normal.xyz, here.normal.w, acc);
    if (acc.weight <= 0.0) {
        return vec4<f32>(current, 1.0);
    }
    let alpha = 1.0 - path.temporal_history;
    let blended = mix(acc.color / acc.weight, current, alpha);
    return vec4<f32>(blended, 1.0);
}
