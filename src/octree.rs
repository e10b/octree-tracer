use cgmath::*;

// First palette colour is empty voxel
// const PALETTE: [u32; 3] = [0x00000000, 0x0000FF00, 0x000000FF];
pub const VOXEL_OFFSET: u32 = 134217728;

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct Voxel {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 0: no authored map, 1: mapped but dark, 2..=7: mapped emission intensity.
    pub emission: u8,
    /// 0: unauthored (matte in the shader). 1..=7: smooth to rough.
    pub roughness: u8,
    /// 0 dielectric, 1 metal.
    pub metal: u8,
    /// Octahedral shading normal for the whole voxel. 0 uses the hit face.
    pub normal: u8,
}

impl Voxel {
    pub fn new(r: u8, g: u8, b: u8) -> Voxel {
        Voxel::surface(r, g, b, 0, 0, 0, 0)
    }

    pub fn with_emission(r: u8, g: u8, b: u8, emission: u8) -> Voxel {
        Voxel::surface(r, g, b, emission, 0, 0, 0)
    }

    pub fn surface(
        r: u8,
        g: u8,
        b: u8,
        emission: u8,
        roughness: u8,
        metal: u8,
        normal: u8,
    ) -> Voxel {
        Voxel {
            r,
            g,
            b,
            emission: emission.min(3),
            roughness: roughness.min(3),
            metal: metal.min(1),
            normal: normal.min(63),
        }
    }

    #[allow(dead_code)]
    pub fn from_value(value: u32) -> Voxel {
        Voxel {
            r: expand5((value >> 11) & 31),
            g: expand6((value >> 5) & 63),
            b: expand5(value & 31),
            emission: ((value >> 16) & 3) as u8,
            roughness: ((value >> 19) & 3) as u8,
            metal: ((value >> 18) & 1) as u8,
            normal: ((value >> 21) & 63) as u8,
        }
    }

    pub fn to_value(&self) -> u32 {
        (VOXEL_OFFSET + self.to_cpu_value()) << 4
    }

    /// 27-bit payload. The GPU word is `(VOXEL_OFFSET + payload) << 4`, so the
    /// payload has to stay below 2^27 or it collides with a child pointer.
    /// Colour is 5-6-5. Then 2-bit emission, metal, 2-bit roughness, and a
    /// 6-bit octahedral normal. 0 in the normal field means "use the hit face".
    pub fn to_cpu_value(&self) -> u32 {
        let r = (self.r as u32) >> 3;
        let g = (self.g as u32) >> 2;
        let b = (self.b as u32) >> 3;
        b | (g << 5)
            | (r << 11)
            | ((self.emission.min(3) as u32) << 16)
            | ((self.metal.min(1) as u32) << 18)
            | ((self.roughness.min(3) as u32) << 19)
            | ((self.normal.min(63) as u32) << 21)
    }
}

/// 0 none, 1 mapped dark, 2 mid, 3 bright. The brightmap sampler still speaks 0..=7.
pub(crate) fn pack_emission(emission: u8) -> u8 {
    match emission {
        0 => 0,
        1 => 1,
        2..=4 => 2,
        _ => 3,
    }
}

const AXIS_NORMALS: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

/// Packs a unit direction into 1..=63. 0 is reserved for "no stored normal".
/// Codes 1..=6 are the exact axes, so a flat floor or wall does not pick up a lean.
pub fn pack_oct_normal(n: [f32; 3]) -> u8 {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1.0e-8);
    let x = n[0] / len;
    let y = n[1] / len;
    let z = n[2] / len;
    if y.abs() > 0.98 && x.abs() < 0.15 && z.abs() < 0.15 {
        return if y > 0.0 { 3 } else { 4 };
    }
    if x.abs() > 0.98 && y.abs() < 0.15 && z.abs() < 0.15 {
        return if x > 0.0 { 1 } else { 2 };
    }
    if z.abs() > 0.98 && x.abs() < 0.15 && y.abs() < 0.15 {
        return if z > 0.0 { 5 } else { 6 };
    }
    let inv = 1.0 / (x.abs() + y.abs() + z.abs());
    let mut u = x * inv;
    let mut v = y * inv;
    if z <= 0.0 {
        let ou = u;
        let ov = v;
        u = (1.0 - ov.abs()) * if ou >= 0.0 { 1.0 } else { -1.0 };
        v = (1.0 - ou.abs()) * if ov >= 0.0 { 1.0 } else { -1.0 };
    }
    let quant = |s: f32| -> u32 {
        ((s.clamp(-1.0, 1.0) * 0.5 + 0.5) * 7.0)
            .round()
            .clamp(0.0, 7.0) as u32
    };
    let idx = (quant(v) * 8 + quant(u)).min(56);
    (idx + 7) as u8
}

/// Inverse of `pack_oct_normal`. `None` when the voxel should use the hit face.
pub fn unpack_oct_normal(code: u8) -> Option<[f32; 3]> {
    if code == 0 || code > 63 {
        return None;
    }
    if code <= 6 {
        return Some(AXIS_NORMALS[(code - 1) as usize]);
    }
    let idx = (code - 7) as u32;
    let u = (idx & 7) as f32 / 7.0 * 2.0 - 1.0;
    let v = (idx >> 3) as f32 / 7.0 * 2.0 - 1.0;
    let mut x = u;
    let mut y = v;
    let mut z = 1.0 - u.abs() - v.abs();
    if z < 0.0 {
        let ox = x;
        let oy = y;
        x = (1.0 - oy.abs()) * if ox >= 0.0 { 1.0 } else { -1.0 };
        y = (1.0 - ox.abs()) * if oy >= 0.0 { 1.0 } else { -1.0 };
    }
    let len = (x * x + y * y + z * z).sqrt().max(1.0e-8);
    Some([x / len, y / len, z / len])
}

fn expand5(v: u32) -> u8 {
    ((v << 3) | (v >> 2)) as u8
}

fn expand6(v: u32) -> u8 {
    ((v << 2) | (v >> 4)) as u8
}

/// Layout (Outdated)
/// ```
/// 01100101 01100101 01100101 01100101
///  ^---- Node: pointer to children, Voxel: palette index
/// ^----- 0: Node, 1: Voxel
/// ```
pub struct Octree {
    pub nodes: Vec<u32>,
    // stays on cpu
    pub positions: Vec<Vector3<f32>>,
    pub hole_stack: Vec<usize>,
    /// (parent leaf, first child) pairs. The children start from the parent's
    /// light and then refine it.
    pub light_copies: Vec<(u32, u32)>,
    /// (parent, first child) pairs from an unload. The children's radiance is
    /// downsampled into the parent, which is a leaf again.
    pub light_folds: Vec<(u32, u32)>,
}

impl Octree {
    pub fn new(mask: [Voxel; 8]) -> Self {
        let mut nodes = Vec::new();
        let mut positions = Vec::new();
        let hole_stack = Vec::new();

        for i in 0..8 {
            nodes.push(mask[i].to_value());
            positions.push(Octree::pos_offset(i, 1));
        }

        Self {
            nodes,
            positions,
            hole_stack,
            light_copies: Vec::new(),
            light_folds: Vec::new(),
        }
    }

    pub fn get_node(&self, index: usize) -> u32 {
        self.nodes[index] >> 4
    }

    pub fn subdivide(&mut self, node: usize, mask: [Voxel; 8], depth: u32) {
        if self.get_node(node) < VOXEL_OFFSET {
            panic!("Node already subdivided!");
        }

        let pos = self.positions[node];
        let child = if let Some(index) = self.hole_stack.pop() {
            self.nodes[node] = create_node(index);
            for i in 0..8 {
                self.nodes[index + i] = mask[i].to_value();
                self.positions[index + i] = pos + Octree::pos_offset(i, depth);
            }
            index
        } else {
            let index = self.nodes.len();
            self.nodes[node] = create_node(index);
            for i in 0..8 {
                self.nodes.push(mask[i].to_value());
                self.positions.push(pos + Octree::pos_offset(i, depth));
            }
            index
        };
        self.light_copies.push((node as u32, child as u32));
    }

    pub fn unsubdivide(&mut self, node: usize) {
        let tnipt = self.get_node(node);
        if tnipt >= VOXEL_OFFSET {
            println!("Node {} not subdivided!", node);
            return;
        }

        self.hole_stack.push(tnipt as usize);
        self.light_folds.push((node as u32, tnipt));

        let pos = self.positions[node];
        if pos == Vector3::zero() {
            panic!("Tried to unsubdivide a node without position!");
        }

        self.nodes[node] = Voxel::new(255, 0, 0).to_value();
    }

    /// Returns (index, depth, pos)
    pub fn find_voxel(
        &self,
        pos: Vector3<f32>,
        max_depth: Option<u32>,
    ) -> (usize, u32, Vector3<f32>) {
        let mut node_index = 0;
        let mut node_pos = Vector3::zero();
        let mut depth = 0;
        loop {
            depth += 1;

            let p = Vector3::new(
                (pos.x >= node_pos.x) as usize,
                (pos.y >= node_pos.y) as usize,
                (pos.z >= node_pos.z) as usize,
            );
            let child_index = p.x * 4 + p.y * 2 + p.z;

            node_pos += Octree::pos_offset(child_index, depth);

            if self.get_node(node_index + child_index) >= VOXEL_OFFSET
                || depth == max_depth.unwrap_or(u32::MAX)
            {
                return (node_index + child_index, depth, node_pos);
            }

            node_index = self.get_node(node_index + child_index) as usize;
        }
    }

    pub fn expanded(&self, size: usize) -> Vec<u32> {
        let mut nodes = self.nodes.clone();
        nodes.extend(std::iter::repeat(0).take(size - self.nodes.len()));

        nodes
    }

    pub fn raw_data(&self) -> &Vec<u32> {
        &self.nodes
    }

    pub fn pos_offset(child_index: usize, depth: u32) -> Vector3<f32> {
        let x = (child_index >> 2) & 1;
        let y = (child_index >> 1) & 1;
        let z = child_index & 1;

        (Vector3::new(x as f32, y as f32, z as f32) * 2.0 - Vector3::new(1.0, 1.0, 1.0))
            / (1 << depth) as f32
    }
}

pub fn create_node(value: usize) -> u32 {
    (value as u32) << 4
}

#[allow(dead_code)]
fn count_bits(mut n: u8) -> usize {
    let mut count = 0;
    while n != 0 {
        n = n & (n - 1);
        count += 1;
    }
    return count;
}

impl std::fmt::Debug for Octree {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Nodes ({}):\n", self.nodes.len())?;
        let mut c = 0;
        for value in &self.nodes {
            let pos = self.positions[c];
            if *value >= VOXEL_OFFSET << 4 {
                write!(
                    f,
                    "  Voxel: {} ({}, {}, {})\n",
                    (*value >> 4) - VOXEL_OFFSET,
                    pos.x,
                    pos.y,
                    pos.z
                )?;
            } else {
                write!(
                    f,
                    "  Node: {} ({}, {}, {})\n",
                    *value >> 4,
                    pos.x,
                    pos.y,
                    pos.z
                )?;
            }

            c += 1;
            if c % 8 == 0 {
                write!(f, "\n")?;
            }
        }

        Ok(())
    }
}

impl std::fmt::Debug for Voxel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "({}, {}, {}; emission {}, rough {}, metal {}, normal {})",
            self.r, self.g, self.b, self.emission, self.roughness, self.metal, self.normal
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Voxel;

    #[test]
    fn voxel_gpu_value_preserves_authored_emission() {
        let voxel = Voxel::surface(12, 34, 56, 6, 3, 1, 45);
        let cpu = voxel.to_cpu_value();
        assert!(cpu < 1 << 27, "payload collided with the node tag");
        assert_eq!(voxel.to_value() >> 4, super::VOXEL_OFFSET + cpu);
        let unpacked = Voxel::from_value(cpu);
        assert_eq!(unpacked.emission, 3);
        assert_eq!(unpacked.roughness, 3);
        assert_eq!(unpacked.metal, 1);
        assert_eq!(unpacked.normal, 45);
        assert_eq!(unpacked.r, super::expand5(12 >> 3));
        assert_eq!(unpacked.g, super::expand6(34 >> 2));
        assert_eq!(unpacked.b, super::expand5(56 >> 3));
    }

    #[test]
    fn diagonal_normal_is_not_an_axis() {
        let code = super::pack_oct_normal([1.0, 0.0, 1.0]);
        let n = super::unpack_oct_normal(code).unwrap();
        assert!(n[0] > 0.4 && n[2] > 0.4, "{n:?}");
        assert!(n[1].abs() < 0.35, "{n:?}");
        assert_eq!(super::pack_oct_normal([0.0, 1.0, 0.0]), 3);
        assert_eq!(super::unpack_oct_normal(3), Some([0.0, 1.0, 0.0]));
        assert_eq!(super::pack_oct_normal(n), code);
        assert!(super::unpack_oct_normal(0).is_none());
    }
}
