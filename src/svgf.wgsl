// SVGF spatial filter (Schied et al., HPG 2017).
//
// The path tracer writes this frame's illumination, normals, albedo and luminance
// moments into textures. A storage-buffer read from this shader does not see that
// data, so the filter only uses textureLoad. Five draws use strides 1, 2, 4, 8
// and 16. The last draw's illumination is remodulated by albedo for display.

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

struct SvgfUniforms {
    step: u32;
    width: u32;
    height: u32;
    // 0: illumination from this frame's texture, variance from moments.
    // 1: illumination and variance from the previous wavelet target.
    mode: u32;
};

[[group(0), binding(0)]]
var<uniform> filter: SvgfUniforms;
[[group(0), binding(1)]]
var illum_tex: texture_2d<f32>;
[[group(0), binding(2)]]
var normal_tex: texture_2d<f32>;
[[group(0), binding(3)]]
var albedo_tex: texture_2d<f32>;
[[group(0), binding(4)]]
var moment_tex: texture_2d<f32>;
[[group(0), binding(5)]]
var filtered: texture_2d<f32>;

let PHI_COLOUR = 4.0;
let PHI_NORMAL = 128.0;

fn luminance(colour: vec3<f32>) -> f32 {
    return dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn b3_spline(tap: i32) -> f32 {
    if (tap == -2 || tap == 2) {
        return 1.0 / 16.0;
    }
    if (tap == -1 || tap == 1) {
        return 1.0 / 4.0;
    }
    return 3.0 / 8.0;
}

fn missing_pixel() -> HistoryPixel {
    return HistoryPixel(
        vec4<f32>(0.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, -1.0),
        vec4<f32>(0.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 0.0),
        vec4<f32>(1.0, 1.0, 1.0, 0.0),
    );
}

fn in_bounds(coord: vec2<i32>) -> bool {
    return coord.x >= 0
        && coord.y >= 0
        && coord.x < i32(filter.width)
        && coord.y < i32(filter.height);
}

fn load_history(coord: vec2<i32>) -> HistoryPixel {
    if (!in_bounds(coord)) {
        return missing_pixel();
    }
    let illum = textureLoad(illum_tex, coord, 0);
    let normal = textureLoad(normal_tex, coord, 0);
    let albedo = textureLoad(albedo_tex, coord, 0);
    let moments = textureLoad(moment_tex, coord, 0);
    return HistoryPixel(
        illum,
        vec4<f32>(0.0, 0.0, 0.0, albedo.a),
        normal,
        moments,
        vec4<f32>(albedo.rgb, 0.0),
    );
}

fn geometry_weight(center: HistoryPixel, sample: HistoryPixel, offset: vec2<f32>, gradient: vec2<f32>) -> f32 {
    if (sample.position.w < 0.0) {
        return 0.0;
    }
    // The pixel's own sample always contributes. A zero normal must not wipe it.
    if (dot(offset, offset) == 0.0) {
        return 1.0;
    }
    let center_hit = center.position.w > 0.5;
    let sample_hit = sample.position.w > 0.5;
    if (center_hit != sample_hit) {
        return 0.0;
    }
    if (!center_hit) {
        return 1.0;
    }
    if (dot(center.normal.xyz, center.normal.xyz) < 0.01) {
        return 1.0;
    }
    let normal_w = pow(max(dot(center.normal.xyz, sample.normal.xyz), 0.0), PHI_NORMAL);
    let depth_delta = abs(sample.normal.w - center.normal.w);
    let expected = abs(dot(gradient, offset)) + 0.002 * center.normal.w + 0.001;
    return normal_w * exp(-depth_delta / expected);
}

fn luminance_weight(center_luma: f32, sample_luma: f32, variance: f32) -> f32 {
    let deviation = PHI_COLOUR * sqrt(max(variance, 0.0)) + 0.001;
    return exp(-abs(sample_luma - center_luma) / deviation);
}

fn depth_gradient(pixel: vec2<i32>, center_depth: f32) -> vec2<f32> {
    let positive_x = load_history(pixel + vec2<i32>(1, 0));
    let negative_x = load_history(pixel + vec2<i32>(-1, 0));
    let positive_y = load_history(pixel + vec2<i32>(0, 1));
    let negative_y = load_history(pixel + vec2<i32>(0, -1));
    var depth_x = center_depth;
    var depth_nx = center_depth;
    var depth_y = center_depth;
    var depth_ny = center_depth;
    if (positive_x.position.w >= 0.0) {
        depth_x = positive_x.normal.w;
    }
    if (negative_x.position.w >= 0.0) {
        depth_nx = negative_x.normal.w;
    }
    if (positive_y.position.w >= 0.0) {
        depth_y = positive_y.normal.w;
    }
    if (negative_y.position.w >= 0.0) {
        depth_ny = negative_y.normal.w;
    }
    return vec2<f32>(depth_x - depth_nx, depth_y - depth_ny) * 0.5;
}

fn estimate_variance(pixel: vec2<i32>, center: HistoryPixel, gradient: vec2<f32>) -> f32 {
    var moment1 = 0.0;
    var moment2 = 0.0;
    var weight_sum = 0.0;
    var y = -3;
    loop {
        if (y > 3) { break; }
        var x = -3;
        loop {
            if (x > 3) { break; }
            let sample = load_history(pixel + vec2<i32>(x, y));
            let weight = geometry_weight(center, sample, vec2<f32>(f32(x), f32(y)), gradient);
            moment1 = moment1 + sample.moments.x * weight;
            moment2 = moment2 + sample.moments.y * weight;
            weight_sum = weight_sum + weight;
            x = x + 1;
        }
        y = y + 1;
    }
    let scale = 1.0 / max(weight_sum, 0.0001);
    let mean = moment1 * scale;
    let second = moment2 * scale;
    return max(0.0, second - mean * mean);
}

struct FilterTap {
    colour: vec3<f32>;
    variance: f32;
    valid: bool;
    geo: HistoryPixel;
};

fn load_tap(coord: vec2<i32>, center_variance: f32) -> FilterTap {
    let geo = load_history(coord);
    if (geo.position.w < 0.0) {
        return FilterTap(vec3<f32>(0.0), 0.0, false, geo);
    }
    if (filter.mode == 0u) {
        return FilterTap(geo.radiance.xyz, center_variance, true, geo);
    }
    let texel = textureLoad(filtered, coord, 0);
    return FilterTap(texel.rgb, texel.a, true, geo);
}

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

[[stage(fragment)]]
fn fs_main(in: VSOut) -> [[location(0)]] vec4<f32> {
    let pixel = vec2<i32>(i32(in.clip.x), i32(in.clip.y));
    let center = load_history(pixel);
    let gradient = depth_gradient(pixel, center.normal.w);
    var center_variance = 0.0;
    if (filter.mode == 0u) {
        center_variance = estimate_variance(pixel, center, gradient);
    } else {
        center_variance = textureLoad(filtered, pixel, 0).a;
    }
    let center_tap = load_tap(pixel, center_variance);
    let center_luma = luminance(center_tap.colour);

    var colour = vec3<f32>(0.0);
    var variance_sum = 0.0;
    var weight_sum = 0.0;
    var y = 0;
    loop {
        if (y > 4) { break; }
        var x = 0;
        loop {
            if (x > 4) { break; }
            let kernel = b3_spline(x - 2) * b3_spline(y - 2);
            let texel_offset = vec2<i32>(x - 2, y - 2) * i32(filter.step);
            let tap = load_tap(pixel + texel_offset, center_variance);
            if (tap.valid) {
                let offset = vec2<f32>(texel_offset);
                let weight = kernel
                    * geometry_weight(center, tap.geo, offset, gradient)
                    * luminance_weight(center_luma, luminance(tap.colour), center_variance);
                colour = colour + tap.colour * weight;
                variance_sum = variance_sum + tap.variance * weight;
                weight_sum = weight_sum + weight;
            }
            x = x + 1;
        }
        y = y + 1;
    }

    let filtered_illumination = colour / max(weight_sum, 0.0001);
    let filtered_variance = variance_sum / max(weight_sum, 0.0001);
    return vec4<f32>(filtered_illumination, filtered_variance);
}

[[stage(fragment)]]
fn fs_modulate(in: VSOut) -> [[location(0)]] vec4<f32> {
    let pixel = vec2<i32>(i32(in.clip.x), i32(in.clip.y));
    let illumination = textureLoad(filtered, pixel, 0).rgb;
    let albedo = textureLoad(albedo_tex, pixel, 0).rgb;
    return vec4<f32>(illumination * albedo, 1.0);
}
