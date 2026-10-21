// pad0 == 0: unload. Downsample the eight children's radiance into the parent.
// pad0 == 1: reload. Seed each child with the parent's radiance and a short
// count, so the coarse light shows immediately and new samples refine it.
// Record layout matches the path tracer: four u32s per node. rgb are the
// summed indirect irradiance in fixed point (2048 units per 1.0). The low
// 7 bits of w are the sample count. The sun bit and frame tag above them
// are dropped, so a folded leaf is gathered again.

struct Records {
    data: [[stride(4)]] array<u32>;
};

struct U32s {
    data: [[stride(4)]] array<u32>;
};

struct FoldUniforms {
    count: u32;
    capacity: u32;
    pad0: u32;
    pad1: u32;
};

[[group(0), binding(0)]]
var<storage, read_write> light: Records;
[[group(0), binding(1)]]
var<storage, read> pairs: U32s;
[[group(0), binding(2)]]
var<uniform> fold: FoldUniforms;

fn pack_rad(value: f32) -> u32 {
    let q = i32(clamp(value, -64.0, 4096.0) * 2048.0);
    return bitcast<u32>(q);
}

fn unpack_rad(value: u32) -> f32 {
    return f32(bitcast<i32>(value)) / 2048.0;
}

fn load_mean(index: u32) -> vec4<f32> {
    let count = light.data[index * 4u + 3u] & 127u;
    if (count == 0u) {
        return vec4<f32>(0.0);
    }
    let inv = 1.0 / f32(count);
    return vec4<f32>(
        unpack_rad(light.data[index * 4u]) * inv,
        unpack_rad(light.data[index * 4u + 1u]) * inv,
        unpack_rad(light.data[index * 4u + 2u]) * inv,
        f32(count),
    );
}

fn store_mean(index: u32, mean: vec3<f32>, count: u32) {
    let w = f32(count);
    light.data[index * 4u] = pack_rad(mean.x * w);
    light.data[index * 4u + 1u] = pack_rad(mean.y * w);
    light.data[index * 4u + 2u] = pack_rad(mean.z * w);
    light.data[index * 4u + 3u] = count;
}

fn store_zero(index: u32) {
    light.data[index * 4u] = 0u;
    light.data[index * 4u + 1u] = 0u;
    light.data[index * 4u + 2u] = 0u;
    light.data[index * 4u + 3u] = 0u;
}

[[stage(compute), workgroup_size(64)]]
fn main([[builtin(global_invocation_id)]] id: vec3<u32>) {
    if (id.x >= fold.count) {
        return;
    }
    let parent = pairs.data[id.x * 2u];
    let child = pairs.data[id.x * 2u + 1u];
    if (parent >= fold.capacity || child + 7u >= fold.capacity) {
        return;
    }
    if (fold.pad0 == 1u) {
        let rec = load_mean(parent);
        var i = 0u;
        if (rec.w <= 0.0) {
            loop {
                if (i >= 8u) { break; }
                store_zero(child + i);
                i = i + 1u;
            }
            return;
        }
        var prior = rec.w;
        if (prior > 8.0) {
            prior = 8.0;
        }
        let count = u32(prior);
        loop {
            if (i >= 8u) { break; }
            store_mean(child + i, rec.xyz, count);
            i = i + 1u;
        }
        return;
    }
    var acc = vec3<f32>(0.0);
    var k = 0u;
    var j = 0u;
    loop {
        if (j >= 8u) { break; }
        let rec = load_mean(child + j);
        if (rec.w > 0.0) {
            acc = acc + rec.xyz;
            k = k + 1u;
        }
        j = j + 1u;
    }
    if (k == 0u) {
        return;
    }
    store_mean(parent, acc / f32(k), 8u);
}
