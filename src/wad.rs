//! E1M1 from a Doom IWAD on disk. The commercial WAD is not part of this repo.
//! Patches and flats come from PLAYPAL. A lump whose bytes are a PNG is used as-is,
//! the way GZDoom accepts PNG textures.

use super::*;
use std::collections::HashMap;
use std::sync::Arc;

/// Corner of the voxel cube, in Doom map units, so room's actors land on the walls.
pub struct MapFrame {
    pub min_x: i32,
    pub min_h: i32,
    pub max_y: i32,
    pub grid: i32,
}

pub struct BuiltMap {
    pub voxels: HashMap<u64, Voxel>,
    pub spawn: [i32; 3],
    pub look: [f32; 3],
    pub grid: i32,
    pub frame: MapFrame,
    pub pictures: crate::play::Pictures,
}

#[derive(Default)]
struct Brightmaps {
    textures: HashMap<String, Arc<image::RgbaImage>>,
    flats: HashMap<String, Arc<image::RgbaImage>>,
}

impl Brightmaps {
    fn load() -> Self {
        let Some(root) = find_brightmap_dir() else {
            println!("No authored brightmaps found; texture emission disabled");
            return Self::default();
        };
        let mut textures = load_brightmap_directory(&root);
        let mut flats = load_brightmap_directory(&root.join("flats"));
        if let Some(definitions) = root.parent() {
            load_brightmap_definitions(
                &mut textures,
                &definitions.join("doom_textures.bm"),
                "texture",
            );
            load_brightmap_definitions(&mut flats, &definitions.join("doom_flats.bm"), "flat");
        }
        println!(
            "Loaded {} texture and {} flat brightmaps from {}",
            textures.len(),
            flats.len(),
            root.display()
        );
        Self { textures, flats }
    }

    fn texture(&self, name: &str) -> Option<&image::RgbaImage> {
        self.textures.get(name).map(AsRef::as_ref)
    }

    fn flat(&self, name: &str) -> Option<&image::RgbaImage> {
        self.flats.get(name).map(AsRef::as_ref)
    }
}

fn find_brightmap_dir() -> Option<std::path::PathBuf> {
    let project = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut bases = Vec::new();
    if let Ok(path) = std::env::var("DOOM_BRIGHTMAPS_DIR") {
        bases.push(std::path::PathBuf::from(path));
    }
    bases.push(project.join("files/gzdoom_brightmaps"));
    bases.push(project.join("../gzdoom_brightmaps"));

    for base in bases {
        for candidate in [
            base.join("bmplus_textures/brightmaps/textures/doom"),
            base.join("brightmaps/textures/doom"),
            base.clone(),
        ] {
            if candidate.is_dir()
                && std::fs::read_dir(&candidate)
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .any(|entry| {
                        entry
                            .path()
                            .extension()
                            .map(|ext| ext.eq_ignore_ascii_case("png"))
                            .unwrap_or(false)
                    })
            {
                return Some(candidate);
            }
        }
    }
    None
}

fn load_brightmap_directory(path: &std::path::Path) -> HashMap<String, Arc<image::RgbaImage>> {
    let mut maps = HashMap::new();
    let Ok(entries) = std::fs::read_dir(path) else {
        return maps;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("png"))
            .unwrap_or(false)
        {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Ok(image) = image::open(&path) {
            maps.insert(name.to_ascii_uppercase(), Arc::new(image.to_rgba8()));
        }
    }
    maps
}

fn load_brightmap_definitions(
    maps: &mut HashMap<String, Arc<image::RgbaImage>>,
    path: &std::path::Path,
    kind: &str,
) {
    let Ok(definitions) = std::fs::read_to_string(path) else {
        return;
    };
    apply_brightmap_definitions(maps, &definitions, kind);
}

fn apply_brightmap_definitions(
    maps: &mut HashMap<String, Arc<image::RgbaImage>>,
    definitions: &str,
    kind: &str,
) {
    for line in definitions.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        let mut words = line.split_whitespace();
        if words.next() != Some("brightmap") || words.next() != Some(kind) {
            continue;
        }
        let Some(material_name) = words.next() else {
            continue;
        };
        let Some(map_start) = line.find("map \"") else {
            continue;
        };
        let map_path = &line[map_start + 5..];
        let Some(map_end) = map_path.find('"') else {
            continue;
        };
        let Some(map_name) = std::path::Path::new(&map_path[..map_end])
            .file_stem()
            .and_then(|name| name.to_str())
        else {
            continue;
        };
        if let Some(image) = maps.get(&map_name.to_ascii_uppercase()).cloned() {
            maps.insert(material_name.to_ascii_uppercase(), image);
        }
    }
}

struct MaterialPaths {
    normal: Option<String>,
    metallic: Option<String>,
    roughness: Option<String>,
    ao: Option<String>,
    glossiness: Option<f32>,
}

struct PbrTex {
    normal: Option<Arc<image::RgbaImage>>,
    metallic: Option<Arc<image::RgbaImage>>,
    roughness: Option<Arc<image::RgbaImage>>,
    ao: Option<Arc<image::RgbaImage>>,
    glossiness: Option<f32>,
}

struct PbrPack {
    materials: HashMap<String, PbrTex>,
}

impl PbrPack {
    fn load(names: &std::collections::HashSet<String>) -> Self {
        let Some(root) = find_pbr_dir() else {
            println!("No DDS PBR pack found; walls stay matte");
            return Self {
                materials: HashMap::new(),
            };
        };
        let Ok(text) = std::fs::read_to_string(root.join("GLDEFS")) else {
            println!("PBR pack at {} has no GLDEFS", root.display());
            return Self {
                materials: HashMap::new(),
            };
        };
        let defs = parse_gldefs(&text);
        let mut cache: HashMap<String, Arc<image::RgbaImage>> = HashMap::new();
        let mut materials = HashMap::new();
        for name in names {
            let Some(paths) = defs.get(&name.to_ascii_uppercase()) else {
                continue;
            };
            let tex = PbrTex {
                normal: load_cached(&root, &paths.normal, &mut cache)
                    .map(|image| Arc::new(demean_tangent(&image))),
                metallic: load_cached(&root, &paths.metallic, &mut cache),
                roughness: load_cached(&root, &paths.roughness, &mut cache),
                ao: load_cached(&root, &paths.ao, &mut cache),
                glossiness: paths.glossiness,
            };
            if tex.normal.is_some()
                || tex.roughness.is_some()
                || tex.metallic.is_some()
                || tex.glossiness.is_some()
            {
                materials.insert(name.to_ascii_uppercase(), tex);
            }
        }
        println!(
            "Loaded {} PBR materials from {}",
            materials.len(),
            root.display()
        );
        Self { materials }
    }

    fn get(&self, name: &str) -> Option<&PbrTex> {
        self.materials.get(&name.to_ascii_uppercase())
    }
}

fn find_pbr_dir() -> Option<std::path::PathBuf> {
    let project = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut bases = Vec::new();
    if let Ok(path) = std::env::var("DOOM_PBR_DIR") {
        bases.push(std::path::PathBuf::from(path));
    }
    bases.push(project.join("files/dds_textures_pbr"));
    bases.push(project.join("../.DDS_TexturesPBR_PLUS"));
    bases.push(project.join("../DDS_TexturesPBR_PLUS"));
    bases.into_iter().find(|base| base.join("GLDEFS").is_file() && base.join("materials").is_dir())
}

fn parse_gldefs(text: &str) -> HashMap<String, MaterialPaths> {
    let mut out = HashMap::new();
    let mut current: Option<(String, MaterialPaths)> = None;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line.to_ascii_lowercase().starts_with("material ") {
            if let Some((name, paths)) = current.take() {
                out.insert(name, paths);
            }
            let mut words = line.split_whitespace();
            words.next();
            words.next();
            let name = words
                .next()
                .unwrap_or_default()
                .trim_matches('"')
                .to_ascii_uppercase();
            if !name.is_empty() {
                current = Some((name, MaterialPaths::empty()));
            }
            continue;
        }
        if line.starts_with('}') {
            if let Some((name, paths)) = current.take() {
                out.insert(name, paths);
            }
            continue;
        }
        let Some((_, paths)) = current.as_mut() else {
            continue;
        };
        if let Some(value) = gldef_value(line, "normal") {
            paths.normal = Some(value);
        } else if let Some(value) = gldef_value(line, "metallic") {
            paths.metallic = Some(value);
        } else if let Some(value) = gldef_value(line, "roughness") {
            paths.roughness = Some(value);
        } else if let Some(value) = gldef_value(line, "ao") {
            paths.ao = Some(value);
        } else if let Some(value) = gldef_value(line, "glossiness") {
            paths.glossiness = value.parse().ok();
        }
    }
    if let Some((name, paths)) = current.take() {
        out.insert(name, paths);
    }
    out
}

impl MaterialPaths {
    fn empty() -> Self {
        Self {
            normal: None,
            metallic: None,
            roughness: None,
            ao: None,
            glossiness: None,
        }
    }
}

fn gldef_value(line: &str, key: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case(key) {
        return None;
    }
    let value = words.next()?.trim_matches('"');
    if value.is_empty() {
        None
    } else {
        Some(value.replace('\\', "/"))
    }
}

fn load_cached(
    root: &std::path::Path,
    path: &Option<String>,
    cache: &mut HashMap<String, Arc<image::RgbaImage>>,
) -> Option<Arc<image::RgbaImage>> {
    let path = path.as_ref()?;
    if path.to_ascii_lowercase().contains("blank") {
        return None;
    }
    if let Some(image) = cache.get(path) {
        return Some(Arc::clone(image));
    }
    let image = Arc::new(load_dds(&root.join(path))?);
    cache.insert(path.clone(), Arc::clone(&image));
    Some(image)
}

fn load_dds(path: &std::path::Path) -> Option<image::RgbaImage> {
    let bytes = read_material(path)?;
    decode_dds(&bytes)
}

fn read_material(path: &std::path::Path) -> Option<Vec<u8>> {
    if let Ok(bytes) = std::fs::read(path) {
        return Some(bytes);
    }
    let parent = path.parent()?;
    let want = path.file_name()?.to_str()?;
    let entry = std::fs::read_dir(parent).ok()?.flatten().find(|entry| {
        entry
            .file_name()
            .to_str()
            .map(|name| name.eq_ignore_ascii_case(want))
            .unwrap_or(false)
    })?;
    std::fs::read(entry.path()).ok()
}

/// Top mip of an uncompressed BGRA or DXT1/3/5 DDS, nearest-reduced so a 2048
/// map does not stay fully resident while the level is voxelized.
fn decode_dds(bytes: &[u8]) -> Option<image::RgbaImage> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " {
        return None;
    }
    let height = u32::from_le_bytes(bytes[12..16].try_into().ok()?);
    let width = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
    if width == 0 || height == 0 {
        return None;
    }
    let fourcc = &bytes[84..88];
    let full = if fourcc == b"DXT1" {
        decode_dxt(&bytes[128..], width, height, 8)?
    } else if fourcc == b"DXT3" || fourcc == b"DXT5" {
        decode_dxt(&bytes[128..], width, height, 16)?
    } else if fourcc == [0, 0, 0, 0] {
        let bitcount = u32::from_le_bytes(bytes[88..92].try_into().ok()?);
        if bitcount != 32 {
            return None;
        }
        let pitch = u32::from_le_bytes(bytes[20..24].try_into().ok()?);
        decode_bgra(&bytes[128..], width, height, pitch)?
    } else {
        return None;
    };
    Some(downsample_rgba(full, width, height))
}

fn decode_bgra(pixels: &[u8], width: u32, height: u32, pitch: u32) -> Option<Vec<u8>> {
    let row = if pitch >= width.saturating_mul(4) {
        pitch as usize
    } else {
        width as usize * 4
    };
    let needed = row.checked_mul(height as usize)?;
    if pixels.len() < needed {
        return None;
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let src = y * row + x * 4;
            let dst = (y * width as usize + x) * 4;
            rgba[dst] = pixels[src + 2];
            rgba[dst + 1] = pixels[src + 1];
            rgba[dst + 2] = pixels[src];
            rgba[dst + 3] = pixels[src + 3];
        }
    }
    Some(rgba)
}

fn decode_dxt(pixels: &[u8], width: u32, height: u32, block_bytes: usize) -> Option<Vec<u8>> {
    let blocks_x = (width as usize + 3) / 4;
    let blocks_y = (height as usize + 3) / 4;
    let needed = blocks_x.checked_mul(blocks_y)?.checked_mul(block_bytes)?;
    if pixels.len() < needed {
        return None;
    }
    let color_at = block_bytes - 8;
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let off = (by * blocks_x + bx) * block_bytes + color_at;
            let colors = dxt_color_block(&pixels[off..off + 8]);
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= width as usize || y >= height as usize {
                        continue;
                    }
                    let dst = (y * width as usize + x) * 4;
                    rgba[dst..dst + 4].copy_from_slice(&colors[py * 4 + px]);
                }
            }
        }
    }
    Some(rgba)
}

fn dxt_color_block(block: &[u8]) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let a = expand565(c0);
    let b = expand565(c1);
    let mix = |left: [u8; 3], lw: u16, right: [u8; 3], rw: u16| {
        [
            ((left[0] as u16 * lw + right[0] as u16 * rw) / (lw + rw)) as u8,
            ((left[1] as u16 * lw + right[1] as u16 * rw) / (lw + rw)) as u8,
            ((left[2] as u16 * lw + right[2] as u16 * rw) / (lw + rw)) as u8,
            255,
        ]
    };
    let colors = if c0 > c1 {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(a, 2, b, 1),
            mix(a, 1, b, 2),
        ]
    } else {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(a, 1, b, 1),
            [0, 0, 0, 0],
        ]
    };
    let mut out = [[0u8; 4]; 16];
    for i in 0..16 {
        out[i] = colors[((bits >> (2 * i)) & 3) as usize];
    }
    out
}

fn expand565(color: u16) -> [u8; 3] {
    let r = ((color >> 11) & 31) as u8;
    let g = ((color >> 5) & 63) as u8;
    let b = (color & 31) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// Drops a map-wide tangent lean so a biased normal becomes local bumps.
fn demean_tangent(image: &image::RgbaImage) -> image::RgbaImage {
    let count = (image.width() as u64) * (image.height() as u64);
    if count == 0 {
        return image.clone();
    }
    let mut sum_x = 0u64;
    let mut sum_y = 0u64;
    for pixel in image.pixels() {
        sum_x += pixel[0] as u64;
        sum_y += pixel[1] as u64;
    }
    let mean_x = (sum_x / count) as i32;
    let mean_y = (sum_y / count) as i32;
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        pixel[0] = (pixel[0] as i32 - mean_x + 128).clamp(0, 255) as u8;
        pixel[1] = (pixel[1] as i32 - mean_y + 128).clamp(0, 255) as u8;
    }
    out
}

fn downsample_rgba(full: Vec<u8>, width: u32, height: u32) -> image::RgbaImage {
    let max_side = 256u32;
    let (out_w, out_h) = if width.max(height) <= max_side {
        (width, height)
    } else {
        let scale = max_side as f32 / width.max(height) as f32;
        (
            ((width as f32 * scale).round() as u32).max(1),
            ((height as f32 * scale).round() as u32).max(1),
        )
    };
    if out_w == width && out_h == height {
        return image::RgbaImage::from_raw(width, height, full)
            .expect("rgba buffer matches the decoded size");
    }
    let mut rgba = vec![0u8; out_w as usize * out_h as usize * 4];
    for y in 0..out_h {
        let sy = (y as u64 * height as u64 / out_h as u64) as usize;
        for x in 0..out_w {
            let sx = (x as u64 * width as u64 / out_w as u64) as usize;
            let src = (sy * width as usize + sx) * 4;
            let dst = (y as usize * out_w as usize + x as usize) * 4;
            rgba[dst..dst + 4].copy_from_slice(&full[src..src + 4]);
        }
    }
    image::RgbaImage::from_raw(out_w, out_h, rgba).expect("rgba buffer matches the reduced size")
}

pub fn try_load_e1m1() -> Option<BuiltMap> {
    let path = find_wad()?;
    println!("Loading Doom from {}", path.display());
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(err) => {
            println!("Could not read {}: {err}", path.display());
            return None;
        }
    };
    match assemble(&data) {
        Ok(map) => Some(map),
        Err(err) => {
            println!("WAD load failed: {err}");
            None
        }
    }
}

pub(crate) fn find_wad() -> Option<std::path::PathBuf> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let names = ["doom1.wad", "DOOM1.WAD", "doom.wad", "DOOM.WAD"];
    let mut cands = Vec::new();
    for name in names {
        cands.push(root.join("files").join(name));
        cands.push(root.join(name));
        cands.push(root.join("../CoolDoom").join(name));
        cands.push(root.join("../room").join(name));
    }
    cands.push(std::path::PathBuf::from(
        "/Users/ethan/Code/CoolDoom/doom1.wad",
    ));
    cands.push(std::path::PathBuf::from(
        "/Users/ethan/Code/CoolDoom/doom.wad",
    ));
    if let Ok(dir) = std::env::var("DOOMWADDIR") {
        for name in names {
            cands.push(std::path::PathBuf::from(&dir).join(name));
        }
    }
    cands.into_iter().find(|path| path.is_file())
}

struct Line {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    flags: i16,
    right: i16,
    left: i16,
}

struct Side {
    xoff: i32,
    yoff: i32,
    upper: String,
    lower: String,
    mid: String,
    sector: i16,
}

struct Sector {
    floor: i16,
    ceil: i16,
    floor_flat: String,
    ceil_flat: String,
    light: i16,
}

struct Thing {
    x: i16,
    y: i16,
    angle: i16,
    kind: i16,
    flags: i16,
}

#[derive(Clone, Copy)]
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

fn assemble(data: &[u8]) -> Result<BuiltMap, String> {
    if data.len() < 12 {
        return Err("file is too small".into());
    }
    let magic = &data[0..4];
    if magic != b"IWAD" && magic != b"PWAD" {
        return Err("not a WAD".into());
    }
    let lump_count = i32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let dir = i32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    if dir + lump_count * 16 > data.len() {
        return Err("directory runs past the end of the file".into());
    }

    let mut lumps: HashMap<String, (usize, usize)> = HashMap::new();
    let mut order: Vec<(String, usize, usize)> = Vec::with_capacity(lump_count);
    let mut flat_names = std::collections::HashSet::new();
    let mut in_flats = false;
    for i in 0..lump_count {
        let at = dir + i * 16;
        let off = i32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        let size = i32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        let name = lump_name(&data[at + 8..at + 16]);
        if off + size <= data.len() {
            lumps.insert(name.clone(), (off, size));
            order.push((name.clone(), off, size));
        }
        if name == "F_START" || name == "FF_START" {
            in_flats = true;
        } else if name == "F_END" || name == "FF_END" {
            in_flats = false;
        } else if in_flats {
            flat_names.insert(name);
        }
    }

    let map_at = order
        .iter()
        .position(|(name, _, _)| name == "E1M1")
        .ok_or("E1M1 is not in this WAD")?;
    let lump_after = |name: &str| -> Result<&[u8], String> {
        let idx = (1..=10)
            .find(|i| {
                order
                    .get(map_at + i)
                    .map(|(n, _, _)| n == name)
                    .unwrap_or(false)
            })
            .ok_or_else(|| format!("{name} is missing from E1M1"))?;
        let (_, off, size) = order[map_at + idx];
        Ok(&data[off..off + size])
    };

    let verts = parse_verts(lump_after("VERTEXES")?)?;
    let lines = parse_lines(lump_after("LINEDEFS")?, &verts)?;
    let sides = parse_sides(lump_after("SIDEDEFS")?)?;
    let mut sectors = parse_sectors(lump_after("SECTORS")?)?;
    let things = parse_things(lump_after("THINGS")?)?;
    let door_sectors: Vec<usize> = sectors
        .iter()
        .enumerate()
        .filter(|(_, sector)| sector.ceil <= sector.floor)
        .map(|(index, _)| index)
        .collect();
    open_closed_sectors(&mut sectors, &lines, &sides);

    let pal = palette(data, &lumps)?;
    let pnames = parse_pnames(data, &lumps)?;
    let textures = parse_textures(data, &lumps, &pnames)?;
    let brightmaps = Brightmaps::load();

    let mut pictures: HashMap<String, Arc<image::RgbaImage>> = HashMap::new();
    let mut needed = std::collections::HashSet::new();
    for side in &sides {
        for name in [&side.upper, &side.lower, &side.mid] {
            if !name.is_empty() && name != "-" {
                needed.insert(name.clone());
            }
        }
    }
    for sector in &sectors {
        needed.insert(sector.floor_flat.clone());
        if sector.ceil_flat != "F_SKY1" {
            needed.insert(sector.ceil_flat.clone());
        }
    }
    for name in &needed {
        if let Some(img) = image_for(name, &flat_names, data, &lumps, &pal, &textures) {
            pictures.insert(name.clone(), Arc::new(img));
        }
    }
    let gray = Arc::new(solid_image([96, 96, 96]));
    println!("Composed {} textures and flats", pictures.len());
    let pbr = PbrPack::load(&needed);

    let min_x = verts.iter().map(|v| v.0).min().unwrap_or(0);
    let max_y = verts.iter().map(|v| v.1).max().unwrap_or(0);
    let min_y = verts.iter().map(|v| v.1).min().unwrap_or(0);
    let max_x = verts.iter().map(|v| v.0).max().unwrap_or(0);
    let min_h = sectors
        .iter()
        .map(|s| s.floor.min(s.ceil) as i32)
        .min()
        .unwrap_or(0);
    let max_h = sectors
        .iter()
        .map(|s| s.floor.max(s.ceil) as i32)
        .max()
        .unwrap_or(0)
        + 160;
    let span = (max_x - min_x + 4)
        .max(max_y - min_y + 4)
        .max(max_h - min_h + 4)
        .max(1);
    let grid = (span as u32).next_power_of_two() as i32;

    let mut voxels: HashMap<u64, Voxel> = HashMap::new();
    let mut edges = vec![Vec::new(); sectors.len()];
    for line in &lines {
        let right = side_sector(&sides, line.right);
        let left = side_sector(&sides, line.left);
        if right == left {
            continue;
        }
        if let Some(s) = right {
            edges[s].push(Edge {
                x0: line.x0,
                y0: line.y0,
                x1: line.x1,
                y1: line.y1,
            });
        }
        if let Some(s) = left {
            edges[s].push(Edge {
                x0: line.x1,
                y0: line.y1,
                x1: line.x0,
                y1: line.y0,
            });
        }
    }

    for (index, sector) in sectors.iter().enumerate() {
        let floor_img = pictures.get(&sector.floor_flat).unwrap_or(&gray);
        let ceil_img = pictures.get(&sector.ceil_flat);
        fill_sector(
            &edges[index],
            sector,
            floor_img,
            ceil_img.map(|img| img.as_ref()),
            brightmaps.flat(&sector.floor_flat),
            brightmaps.flat(&sector.ceil_flat),
            &pbr,
            min_x,
            min_h,
            max_y,
            grid,
            &mut voxels,
        );
    }
    println!("Floors and ceilings: {} voxels", voxels.len());

    for line in &lines {
        draw_line(
            line,
            &sides,
            &sectors,
            &pictures,
            &gray,
            &brightmaps,
            &pbr,
            min_x,
            min_h,
            max_y,
            grid,
            &mut voxels,
        );
    }
    println!("After walls: {} voxels", voxels.len());

    let mut pending = Vec::new();
    let mut clips = HashMap::new();
    let mut spawn_thing = None;
    let n = grid as f32;
    let to_world = |v: f32| -1.0 + (v + 0.5) * 2.0 / n;
    for thing in &things {
        if thing.kind == 1 && spawn_thing.is_none() {
            spawn_thing = Some(thing);
        }
        if is_marker(thing.kind) || thing.flags & 16 != 0 || thing.flags & 4 == 0 {
            continue;
        }
        let Some((sprite, frame)) = sprite_of(thing.kind) else {
            continue;
        };
        let Some(floor) = find_floor(&edges, &sectors, thing.x as f32, thing.y as f32) else {
            continue;
        };
        remember_clip(&mut clips, data, &lumps, &pal, sprite, frame);
        for (name, letters) in extra_frames(thing.kind) {
            for letter in letters.bytes() {
                remember_clip(&mut clips, data, &lumps, &pal, name, letter);
            }
        }
        let vx = thing.x as f32 - min_x as f32;
        let vy = (floor as i32 - min_h) as f32 + 0.5;
        let vz = max_y as f32 - thing.y as f32;
        pending.push(crate::play::PendingPlane {
            feet: [to_world(vx), to_world(vy), to_world(vz)],
            kind: thing.kind,
            sprite,
            frame,
        });
    }
    for (name, letters) in [
        ("PISG", "ABC"),
        ("PISF", "A"),
        ("SHTG", "ABCD"),
        ("SHTF", "AB"),
        ("CHGG", "AB"),
        ("CHGF", "AB"),
        ("MISG", "AB"),
        ("MISF", "ABCD"),
        ("PUFF", "ABCD"),
        ("BAL1", "ABCDE"),
        ("MISL", "ABCD"),
        ("BEXP", "ABCDE"),
    ] {
        for letter in letters.bytes() {
            remember_clip(&mut clips, data, &lumps, &pal, name, letter);
        }
    }
    let guns = load_guns();
    println!(
        "{} sprite planes, {} clips, {} held guns",
        pending.len(),
        clips.len(),
        guns.len()
    );
    let doors = door_faces(
        &lines,
        &sides,
        &sectors,
        &door_sectors,
        &pictures,
        min_x,
        min_h,
        max_y,
        grid,
    );
    println!("{} door planes", doors.len());
    let pictures = crate::play::pack(pending, clips.into_values().collect(), guns, doors, 2.0 / n);

    let spawn_thing = spawn_thing.ok_or("E1M1 has no player 1 start")?;
    let floor =
        find_floor(&edges, &sectors, spawn_thing.x as f32, spawn_thing.y as f32).unwrap_or(0);
    // One cell above the floor slab, then the usual 41-unit eye.
    let spawn = [
        spawn_thing.x as i32 - min_x,
        floor as i32 - min_h + 42,
        max_y - spawn_thing.y as i32,
    ];
    let aim = (spawn_thing.angle as f32).to_radians();
    let look = [aim.cos(), 0.0, -aim.sin()];
    println!(
        "Player start at voxel ({}, {}, {}), grid {grid}",
        spawn[0], spawn[1], spawn[2]
    );

    Ok(BuiltMap {
        voxels,
        spawn,
        look,
        grid,
        frame: MapFrame {
            min_x,
            min_h,
            max_y,
            grid,
        },
        pictures,
    })
}

fn load_guns() -> Vec<crate::play::PendingGun> {
    let names = |sprite: &str, letters: &str| -> Vec<String> {
        letters
            .bytes()
            .map(|frame| format!("{sprite}{}", frame as char))
            .collect()
    };
    vec![
        crate::play::PendingGun {
            label: "PISTOL",
            frames: names("PISG", "ABC"),
            flash: names("PISF", "A"),
        },
        crate::play::PendingGun {
            label: "SHOTGUN",
            frames: names("SHTG", "ABCD"),
            flash: names("SHTF", "AB"),
        },
        crate::play::PendingGun {
            label: "CHAINGUN",
            frames: names("CHGG", "AB"),
            flash: names("CHGF", "AB"),
        },
        crate::play::PendingGun {
            label: "ROCKET",
            frames: names("MISG", "AB"),
            flash: names("MISF", "ABCD"),
        },
    ]
}

fn extra_frames(kind: i16) -> &'static [(&'static str, &'static str)] {
    match kind {
        3004 | 18 => &[("POSS", "ABCDEFGHIJKL")],
        9 | 19 => &[("SPOS", "ABCDEFGHIJKL")],
        3001 | 20 => &[("TROO", "ABCDEFGHIJKLMNO")],
        3002 | 58 | 21 => &[("SARG", "ABCDEFGHIJKL")],
        2035 => &[("BAR1", "AB"), ("BEXP", "ABCDE")],
        _ => &[],
    }
}

fn remember_clip(
    clips: &mut HashMap<String, crate::play::PendingClip>,
    data: &[u8],
    lumps: &HashMap<String, (usize, usize)>,
    pal: &[[u8; 3]; 256],
    sprite: &str,
    frame: u8,
) {
    let name = format!("{sprite}{}", frame as char);
    if clips.contains_key(&name) {
        return;
    }
    let Some(bytes) = sprite_bytes(data, lumps, sprite, frame) else {
        return;
    };
    let Some(pic) = decode_picture(bytes, pal) else {
        return;
    };
    clips.insert(
        name.clone(),
        crate::play::PendingClip {
            name,
            image: pic.image,
            left: pic.left,
            top: pic.top,
        },
    );
}

fn lump_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end])
        .trim()
        .to_ascii_uppercase()
}

fn i16_at(data: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([data[at], data[at + 1]])
}

fn parse_verts(data: &[u8]) -> Result<Vec<(i32, i32)>, String> {
    if data.len() % 4 != 0 {
        return Err("VERTEXES size is not a multiple of 4".into());
    }
    let mut verts = Vec::with_capacity(data.len() / 4);
    for at in (0..data.len()).step_by(4) {
        verts.push((i16_at(data, at) as i32, i16_at(data, at + 2) as i32));
    }
    Ok(verts)
}

fn parse_lines(data: &[u8], verts: &[(i32, i32)]) -> Result<Vec<Line>, String> {
    if data.len() % 14 != 0 {
        return Err("LINEDEFS size is not a multiple of 14".into());
    }
    let mut lines = Vec::with_capacity(data.len() / 14);
    for at in (0..data.len()).step_by(14) {
        let v1 = i16_at(data, at) as usize;
        let v2 = i16_at(data, at + 2) as usize;
        if v1 >= verts.len() || v2 >= verts.len() {
            continue;
        }
        lines.push(Line {
            x0: verts[v1].0 as f32,
            y0: verts[v1].1 as f32,
            x1: verts[v2].0 as f32,
            y1: verts[v2].1 as f32,
            flags: i16_at(data, at + 4),
            right: i16_at(data, at + 10),
            left: i16_at(data, at + 12),
        });
    }
    Ok(lines)
}

fn parse_sides(data: &[u8]) -> Result<Vec<Side>, String> {
    if data.len() % 30 != 0 {
        return Err("SIDEDEFS size is not a multiple of 30".into());
    }
    let mut sides = Vec::with_capacity(data.len() / 30);
    for at in (0..data.len()).step_by(30) {
        sides.push(Side {
            xoff: i16_at(data, at) as i32,
            yoff: i16_at(data, at + 2) as i32,
            upper: lump_name(&data[at + 4..at + 12]),
            lower: lump_name(&data[at + 12..at + 20]),
            mid: lump_name(&data[at + 20..at + 28]),
            sector: i16_at(data, at + 28),
        });
    }
    Ok(sides)
}

fn parse_sectors(data: &[u8]) -> Result<Vec<Sector>, String> {
    if data.len() % 26 != 0 {
        return Err("SECTORS size is not a multiple of 26".into());
    }
    let mut sectors = Vec::with_capacity(data.len() / 26);
    for at in (0..data.len()).step_by(26) {
        sectors.push(Sector {
            floor: i16_at(data, at),
            ceil: i16_at(data, at + 2),
            floor_flat: lump_name(&data[at + 4..at + 12]),
            ceil_flat: lump_name(&data[at + 12..at + 20]),
            light: i16_at(data, at + 20),
        });
    }
    Ok(sectors)
}

fn parse_things(data: &[u8]) -> Result<Vec<Thing>, String> {
    if data.len() % 10 != 0 {
        return Err("THINGS size is not a multiple of 10".into());
    }
    let mut things = Vec::with_capacity(data.len() / 10);
    for at in (0..data.len()).step_by(10) {
        things.push(Thing {
            x: i16_at(data, at),
            y: i16_at(data, at + 2),
            angle: i16_at(data, at + 4),
            kind: i16_at(data, at + 6),
            flags: i16_at(data, at + 8),
        });
    }
    Ok(things)
}

fn side_sector(sides: &[Side], index: i16) -> Option<usize> {
    if index < 0 {
        return None;
    }
    let side = sides.get(index as usize)?;
    if side.sector < 0 {
        None
    } else {
        Some(side.sector as usize)
    }
}

/// Closed door sectors have the ceiling sitting on the floor. Lift them so the
/// octree doorway stays air. The slab itself is a plane from `door_faces`.
fn open_closed_sectors(sectors: &mut [Sector], lines: &[Line], sides: &[Side]) {
    let mut adj = vec![Vec::new(); sectors.len()];
    for line in lines {
        let right = side_sector(sides, line.right);
        let left = side_sector(sides, line.left);
        if let (Some(a), Some(b)) = (right, left) {
            if a != b && a < sectors.len() && b < sectors.len() {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
    }
    let mut opened = 0;
    for _ in 0..3 {
        let ceils: Vec<i16> = sectors.iter().map(|s| s.ceil).collect();
        let mut progress = false;
        for i in 0..sectors.len() {
            if sectors[i].ceil > sectors[i].floor {
                continue;
            }
            let mut target: Option<i16> = None;
            for &n in &adj[i] {
                if ceils[n] > sectors[i].floor {
                    target = Some(target.map_or(ceils[n], |t| t.min(ceils[n])));
                }
            }
            if let Some(ceil) = target {
                sectors[i].ceil = ceil;
                opened += 1;
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    if opened > 0 {
        println!("Raised {opened} closed sectors so doorways can be walked");
    }
}

/// The moving slab of a door: the upper texture that used to fill the closed
/// opening, drawn as a quad on the room side of each door linedef.
fn door_faces(
    lines: &[Line],
    sides: &[Side],
    sectors: &[Sector],
    door_sectors: &[usize],
    pictures: &HashMap<String, Arc<image::RgbaImage>>,
    min_x: i32,
    min_h: i32,
    max_y: i32,
    grid: i32,
) -> Vec<crate::play::PendingDoor> {
    let n = grid as f32;
    let to_world = |v: f32| -1.0 + (v + 0.5) * 2.0 / n;
    let mut out = Vec::new();
    for line in lines {
        let right = side_sector(sides, line.right);
        let left = side_sector(sides, line.left);
        let faces = [(true, right, left), (false, left, right)];
        for (front_is_right, front, back) in faces {
            let (Some(front), Some(back)) = (front, back) else {
                continue;
            };
            if door_sectors.contains(&front) || !door_sectors.contains(&back) {
                continue;
            }
            if front >= sectors.len() || back >= sectors.len() {
                continue;
            }
            let side_index = if front_is_right {
                line.right
            } else {
                line.left
            };
            let Some(side) = sides.get(side_index as usize) else {
                continue;
            };
            if side.upper.is_empty() || side.upper == "-" {
                continue;
            }
            let Some(tex) = pictures.get(&side.upper) else {
                continue;
            };
            let floor = sectors[back].floor as i32;
            let open = sectors[back].ceil as i32;
            let top = open.min(sectors[front].ceil as i32);
            if top <= floor {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = if front_is_right {
                (line.x0, line.y0, line.x1, line.y1)
            } else {
                (line.x1, line.y1, line.x0, line.y0)
            };
            let dx = x1 - x0;
            let dy = y1 - y0;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1.0 {
                continue;
            }
            // Sit the slab just inside the room so it clears the jamb voxels.
            let shove = 0.6 / len;
            let nx = dy * shove;
            let ny = -dx * shove;
            x0 += nx;
            y0 += ny;
            x1 += nx;
            y1 += ny;
            let h = (top - floor) as u32;
            let px_w = len.round().max(1.0) as u32;
            let unpeg_top = line.flags & 16 != 0;
            let peg = if unpeg_top {
                Peg::Top(sectors[front].ceil as i32)
            } else {
                Peg::Bottom(floor)
            };
            let light = sectors[front].light;
            let mut image = image::RgbaImage::new(px_w, h);
            for x in 0..px_w {
                let u = x as i32 + side.xoff;
                for row in 0..h {
                    let map_h = top - 1 - row as i32;
                    let v = peg.row(map_h, tex.height() as i32, side.yoff);
                    let rgb = lit(sample(tex, u, v), light);
                    image.put_pixel(x, row, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
                }
            }
            let bottom = (floor - min_h) as f32 + 0.5;
            let ay = to_world(bottom);
            out.push(crate::play::PendingDoor {
                sector: back as u16,
                a: [to_world(x0 - min_x as f32), ay, to_world(max_y as f32 - y0)],
                b: [to_world(x1 - min_x as f32), ay, to_world(max_y as f32 - y1)],
                height: h as f32 * 2.0 / n,
                image,
            });
        }
    }
    out
}

fn lit(rgb: [u8; 3], light: i16) -> [u8; 3] {
    let factor = (light as f32 / 255.0).clamp(0.25, 1.0);
    rgb.map(|c| (c as f32 * factor).round() as u8)
}

fn palette(data: &[u8], lumps: &HashMap<String, (usize, usize)>) -> Result<[[u8; 3]; 256], String> {
    let (off, size) = lumps.get("PLAYPAL").ok_or("PLAYPAL is missing")?;
    if *size < 768 || off + 768 > data.len() {
        return Err("PLAYPAL is too small".into());
    }
    let mut pal = [[0u8; 3]; 256];
    for i in 0..256 {
        let at = off + i * 3;
        pal[i] = [data[at], data[at + 1], data[at + 2]];
    }
    Ok(pal)
}

fn parse_pnames(
    data: &[u8],
    lumps: &HashMap<String, (usize, usize)>,
) -> Result<Vec<String>, String> {
    let (off, size) = lumps.get("PNAMES").copied().unwrap_or((0, 0));
    if size < 4 || off + size > data.len() {
        return Ok(Vec::new());
    }
    let count = i32::from_le_bytes(data[off..off + 4].try_into().unwrap()).max(0) as usize;
    let mut names = Vec::with_capacity(count);
    for i in 0..count {
        let at = off + 4 + i * 8;
        if at + 8 > off + size {
            break;
        }
        names.push(lump_name(&data[at..at + 8]));
    }
    Ok(names)
}

struct TexDef {
    w: i32,
    h: i32,
    patches: Vec<(i32, i32, usize)>,
}

fn parse_textures(
    data: &[u8],
    lumps: &HashMap<String, (usize, usize)>,
    pnames: &[String],
) -> Result<HashMap<String, TexDef>, String> {
    let mut defs = HashMap::new();
    let Some((off, size)) = lumps.get("TEXTURE1").copied() else {
        return Ok(defs);
    };
    if size < 4 || off + size > data.len() {
        return Ok(defs);
    }
    let tex = &data[off..off + size];
    let count = i32::from_le_bytes(tex[0..4].try_into().unwrap()).max(0) as usize;
    for i in 0..count {
        let dir_at = 4 + i * 4;
        if dir_at + 4 > tex.len() {
            break;
        }
        let start = i32::from_le_bytes(tex[dir_at..dir_at + 4].try_into().unwrap()) as usize;
        if start + 22 > tex.len() {
            continue;
        }
        let name = lump_name(&tex[start..start + 8]);
        let w = i16_at(tex, start + 12) as i32;
        let h = i16_at(tex, start + 14) as i32;
        let patches_n = i16_at(tex, start + 20) as usize;
        let mut patches = Vec::with_capacity(patches_n);
        for p in 0..patches_n {
            let at = start + 22 + p * 10;
            if at + 6 > tex.len() {
                break;
            }
            let ox = i16_at(tex, at) as i32;
            let oy = i16_at(tex, at + 2) as i32;
            let index = i16_at(tex, at + 4) as usize;
            if index < pnames.len() {
                patches.push((ox, oy, index));
            }
        }
        if w > 0 && h > 0 && w <= 2048 && h <= 2048 {
            defs.insert(name, TexDef { w, h, patches });
        }
    }
    Ok(defs)
}

fn image_for(
    name: &str,
    flats: &std::collections::HashSet<String>,
    data: &[u8],
    lumps: &HashMap<String, (usize, usize)>,
    pal: &[[u8; 3]; 256],
    textures: &HashMap<String, TexDef>,
) -> Option<image::RgbaImage> {
    if let Some(png) = png_lump(name, data, lumps) {
        return Some(png);
    }
    if flats.contains(name) {
        if let Some((off, size)) = lumps.get(name) {
            if *size >= 4096 {
                return decode_flat(&data[*off..*off + 4096], pal);
            }
        }
    }
    if let Some(def) = textures.get(name) {
        return compose_texture(def, data, lumps, pal);
    }
    if let Some((off, size)) = lumps.get(name) {
        return decode_picture(&data[*off..*off + *size], pal).map(|pic| pic.image);
    }
    None
}

fn png_lump(
    name: &str,
    data: &[u8],
    lumps: &HashMap<String, (usize, usize)>,
) -> Option<image::RgbaImage> {
    let (off, size) = lumps.get(name).copied()?;
    let bytes = data.get(off..off + size)?;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        image::load_from_memory(bytes)
            .ok()
            .map(|img| img.to_rgba8())
    } else {
        None
    }
}

fn compose_texture(
    def: &TexDef,
    data: &[u8],
    lumps: &HashMap<String, (usize, usize)>,
    pal: &[[u8; 3]; 256],
) -> Option<image::RgbaImage> {
    let mut img = image::RgbaImage::new(def.w as u32, def.h as u32);
    let pnames = pnames_from(data, lumps);
    for &(ox, oy, index) in &def.patches {
        let Some(name) = pnames.get(index) else {
            continue;
        };
        let Some((off, size)) = lumps.get(name).copied() else {
            continue;
        };
        let Some(bytes) = data.get(off..off + size) else {
            continue;
        };
        let patch = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            image::load_from_memory(bytes).ok().map(|im| im.to_rgba8())
        } else {
            decode_picture(bytes, pal).map(|pic| pic.image)
        };
        if let Some(patch) = patch {
            blit(&mut img, &patch, ox, oy);
        }
    }
    Some(img)
}

fn pnames_from(data: &[u8], lumps: &HashMap<String, (usize, usize)>) -> Vec<String> {
    parse_pnames(data, lumps).unwrap_or_default()
}

fn blit(dst: &mut image::RgbaImage, src: &image::RgbaImage, ox: i32, oy: i32) {
    for y in 0..src.height() {
        for x in 0..src.width() {
            let pixel = src.get_pixel(x, y).0;
            if pixel[3] == 0 {
                continue;
            }
            let dx = ox + x as i32;
            let dy = oy + y as i32;
            if dx >= 0 && dy >= 0 && (dx as u32) < dst.width() && (dy as u32) < dst.height() {
                dst.put_pixel(dx as u32, dy as u32, image::Rgba(pixel));
            }
        }
    }
}

struct Picture {
    image: image::RgbaImage,
    left: i32,
    top: i32,
}

fn decode_picture(data: &[u8], pal: &[[u8; 3]; 256]) -> Option<Picture> {
    if data.len() < 8 || data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let w = i16_at(data, 0) as i32;
    let h = i16_at(data, 2) as i32;
    let left = i16_at(data, 4) as i32;
    let top = i16_at(data, 6) as i32;
    if w <= 0 || h <= 0 || w > 1024 || h > 1024 {
        return None;
    }
    if data.len() < 8 + w as usize * 4 {
        return None;
    }
    let mut img = image::RgbaImage::new(w as u32, h as u32);
    for x in 0..w as usize {
        let col = u32::from_le_bytes(data[8 + x * 4..12 + x * 4].try_into().unwrap()) as usize;
        if col >= data.len() {
            continue;
        }
        let mut ofs = col;
        let mut last_delta = -1i32;
        for _ in 0..h + 8 {
            if ofs >= data.len() {
                break;
            }
            let topdelta = data[ofs] as i32;
            if topdelta == 255 {
                break;
            }
            if topdelta <= last_delta || ofs + 1 >= data.len() {
                break;
            }
            last_delta = topdelta;
            let length = data[ofs + 1] as usize;
            let start = ofs + 3;
            if start + length > data.len() {
                break;
            }
            for i in 0..length {
                let y = topdelta as usize + i;
                if y < h as usize {
                    let rgb = pal[data[start + i] as usize];
                    img.put_pixel(
                        x as u32,
                        y as u32,
                        image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
                    );
                }
            }
            ofs = start + length + 1;
        }
    }
    Some(Picture {
        image: img,
        left,
        top,
    })
}

fn decode_flat(data: &[u8], pal: &[[u8; 3]; 256]) -> Option<image::RgbaImage> {
    if data.len() < 4096 {
        return None;
    }
    let mut img = image::RgbaImage::new(64, 64);
    for i in 0..4096 {
        let rgb = pal[data[i] as usize];
        img.put_pixel(
            (i % 64) as u32,
            (i / 64) as u32,
            image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
        );
    }
    Some(img)
}

fn solid_image(rgb: [u8; 3]) -> image::RgbaImage {
    let mut img = image::RgbaImage::new(1, 1);
    img.put_pixel(0, 0, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    img
}

fn sample(img: &image::RgbaImage, u: i32, v: i32) -> [u8; 3] {
    let pixel = sample_rgba(img, u, v);
    if pixel[3] == 0 {
        [80, 80, 80]
    } else {
        [pixel[0], pixel[1], pixel[2]]
    }
}

fn sample_rgba(img: &image::RgbaImage, u: i32, v: i32) -> [u8; 4] {
    let w = img.width() as i32;
    let h = img.height() as i32;
    if w <= 0 || h <= 0 {
        return [80, 80, 80, 255];
    }
    img.get_pixel(u.rem_euclid(w) as u32, v.rem_euclid(h) as u32)
        .0
}

fn shade(rgb: [u8; 3], light: i16, emission: u8, surface: Surface) -> Voxel {
    let factor = (light as f32 / 255.0).clamp(0.25, 1.0);
    let ch = |c: u8| {
        let v = (c as f32 * factor).round() as u8;
        if v == 0 {
            1
        } else {
            v
        }
    };
    Voxel::surface(
        ch(rgb[0]),
        ch(rgb[1]),
        ch(rgb[2]),
        crate::octree::pack_emission(emission),
        surface.roughness,
        surface.metal,
        surface.normal,
    )
}

struct Surface {
    roughness: u8,
    metal: u8,
    /// 0 uses the hit face. Anything else is the voxel's shading normal.
    normal: u8,
}

struct Sampled {
    roughness: u8,
    metal: u8,
    /// Tangent-space normal from the map. None keeps the surface direction.
    tangent: Option<[f32; 3]>,
}

fn sample_map(map: &image::RgbaImage, u: i32, v: i32, src_w: i32, src_h: i32) -> [u8; 4] {
    let src_w = src_w.max(1);
    let src_h = src_h.max(1);
    let x = (u.rem_euclid(src_w) as u32 * map.width() / src_w as u32).min(map.width() - 1);
    let y = (v.rem_euclid(src_h) as u32 * map.height() / src_h as u32).min(map.height() - 1);
    map.get_pixel(x, y).0
}

fn roughness_code(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    if value < 0.34 {
        1
    } else if value < 0.67 {
        2
    } else {
        3
    }
}

fn normalize3(n: [f32; 3]) -> [f32; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1.0e-8);
    [n[0] / len, n[1] / len, n[2] / len]
}

/// Bakes the surface direction, plus the texel normal map, into one voxel normal.
fn orient(normal: [f32; 3], tangent: [f32; 3], bitangent: [f32; 3], sampled: &Sampled) -> Surface {
    let world = match sampled.tangent {
        Some([x, y, z]) => normalize3([
            tangent[0] * x + bitangent[0] * y + normal[0] * z,
            tangent[1] * x + bitangent[1] * y + normal[1] * z,
            tangent[2] * x + bitangent[2] * y + normal[2] * z,
        ]),
        None => normal,
    };
    Surface {
        roughness: sampled.roughness,
        metal: sampled.metal,
        normal: crate::octree::pack_oct_normal(world),
    }
}

/// Samples the pack's tangent-space normal, roughness, and metal at this texel.
/// Missing maps leave the material matte. The caller still supplies the surface direction.
fn surface_of(
    src: &image::RgbaImage,
    u: i32,
    v: i32,
    maps: Option<&PbrTex>,
) -> (f32, Sampled) {
    let Some(maps) = maps else {
        return (1.0, Sampled { roughness: 0, metal: 0, tangent: None });
    };
    let src_w = src.width() as i32;
    let src_h = src.height() as i32;
    let tangent = maps.normal.as_ref().map(|normal| {
        let pixel = sample_map(normal, u, v, src_w, src_h);
        let nx = pixel[0] as f32 / 255.0 * 2.0 - 1.0;
        let ny = pixel[1] as f32 / 255.0 * 2.0 - 1.0;
        let nz = (pixel[2] as f32 / 255.0 * 2.0 - 1.0).max(0.05);
        normalize3([nx, ny, nz])
    });
    let metal = maps
        .metallic
        .as_ref()
        .map(|metallic| {
            let pixel = sample_map(metallic, u, v, src_w, src_h);
            if pixel[0] > 127 { 1 } else { 0 }
        })
        .unwrap_or(0);
    let roughness = if let Some(rough) = &maps.roughness {
        let pixel = sample_map(rough, u, v, src_w, src_h);
        // The pack's roughness is often satin-dark. Lift dielectrics so a
        // mid-grey texel does not become a tight highlight.
        let mut value = pixel[0] as f32 / 255.0;
        if metal == 0 {
            value = value.sqrt();
        }
        roughness_code(value)
    } else if let Some(gloss) = maps.glossiness {
        let value = 1.0 - (gloss / 100.0).clamp(0.0, 1.0);
        roughness_code(if metal == 0 { value.sqrt() } else { value })
    } else if maps.normal.is_some() || maps.metallic.is_some() {
        roughness_code(0.55)
    } else {
        0
    };
    let ao = maps
        .ao
        .as_ref()
        .map(|ao| sample_map(ao, u, v, src_w, src_h)[0] as f32 / 255.0)
        .unwrap_or(1.0)
        .clamp(0.25, 1.0);
    (ao, Sampled { roughness, metal, tangent })
}

fn shade_rgb(rgb: [u8; 3], ao: f32) -> [u8; 3] {
    rgb.map(|channel| ((channel as f32 * ao).round() as u8).max(1))
}

fn sample_emission(
    brightmap: Option<&image::RgbaImage>,
    source: &image::RgbaImage,
    u: i32,
    v: i32,
) -> u8 {
    let Some(map) = brightmap else {
        return 0;
    };
    let sw = source.width().max(1) as i32;
    let sh = source.height().max(1) as i32;
    let mw = map.width().max(1);
    let mh = map.height().max(1);
    let x = (u.rem_euclid(sw) as u32 * mw / sw as u32).min(mw - 1);
    let y = (v.rem_euclid(sh) as u32 * mh / sh as u32).min(mh - 1);
    let pixel = map.get_pixel(x, y).0;
    let mask = pixel[0].max(pixel[1]).max(pixel[2]) as u32 * pixel[3] as u32 / 255;
    if mask == 0 {
        1
    } else {
        (1 + (mask * 6 + 254) / 255).min(7) as u8
    }
}

fn paint(
    voxels: &mut HashMap<u64, Voxel>,
    grid: i32,
    min_x: i32,
    min_h: i32,
    max_y: i32,
    wad_x: i32,
    height: i32,
    wad_y: i32,
    rgb: [u8; 3],
    light: i16,
    emission: u8,
    surface: Surface,
) {
    let x = wad_x - min_x;
    let y = height - min_h;
    let z = max_y - wad_y;
    if x < 0 || y < 0 || z < 0 || x >= grid || y >= grid || z >= grid {
        return;
    }
    voxels.insert(pack(x, y, z), shade(rgb, light, emission, surface));
}

fn pack(x: i32, y: i32, z: i32) -> u64 {
    (x as u64) | ((y as u64) << 16) | ((z as u64) << 32)
}

fn fill_sector(
    edges: &[Edge],
    sector: &Sector,
    floor_img: &image::RgbaImage,
    ceil_img: Option<&image::RgbaImage>,
    floor_brightmap: Option<&image::RgbaImage>,
    ceil_brightmap: Option<&image::RgbaImage>,
    pbr: &PbrPack,
    min_x: i32,
    min_h: i32,
    max_y: i32,
    grid: i32,
    voxels: &mut HashMap<u64, Voxel>,
) {
    if edges.is_empty() {
        return;
    }
    let mut y0 = i32::MAX;
    let mut y1 = i32::MIN;
    for edge in edges {
        y0 = y0.min(edge.y0.floor() as i32).min(edge.y1.floor() as i32);
        y1 = y1.max(edge.y0.ceil() as i32).max(edge.y1.ceil() as i32);
    }
    let sky = sector.ceil_flat == "F_SKY1";
    for y in y0..y1 {
        let sy = y as f32 + 0.5;
        let mut xs = Vec::new();
        for edge in edges {
            if (edge.y0 > sy) == (edge.y1 > sy) || (edge.y1 - edge.y0).abs() < 1e-6 {
                continue;
            }
            let t = (sy - edge.y0) / (edge.y1 - edge.y0);
            xs.push(edge.x0 + t * (edge.x1 - edge.x0));
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut i = 0;
        while i + 1 < xs.len() {
            let a = xs[i];
            let b = xs[i + 1];
            let mut x = a.floor() as i32;
            let end = b.ceil() as i32;
            while x < end {
                let cx = x as f32 + 0.5;
                if cx >= a && cx < b {
                    let emission = sample_emission(floor_brightmap, floor_img, x, y);
                    let (ao, sampled) = surface_of(floor_img, x, y, pbr.get(&sector.floor_flat));
                    let rgb = shade_rgb(sample(floor_img, x, y), ao);
                    paint(
                        voxels,
                        grid,
                        min_x,
                        min_h,
                        max_y,
                        x,
                        sector.floor as i32,
                        y,
                        rgb,
                        sector.light,
                        emission,
                        orient([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], &sampled),
                    );
                    if !sky {
                        if let Some(ceil) = ceil_img {
                            let emission = sample_emission(ceil_brightmap, ceil, x, y);
                            let (ao, sampled) = surface_of(ceil, x, y, pbr.get(&sector.ceil_flat));
                            let rgb = shade_rgb(sample(ceil, x, y), ao);
                            paint(
                                voxels,
                                grid,
                                min_x,
                                min_h,
                                max_y,
                                x,
                                sector.ceil as i32,
                                y,
                                rgb,
                                sector.light,
                                emission,
                                orient([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], &sampled),
                            );
                        }
                    }
                }
                x += 1;
            }
            i += 2;
        }
    }
}

fn draw_line(
    line: &Line,
    sides: &[Side],
    sectors: &[Sector],
    pictures: &HashMap<String, Arc<image::RgbaImage>>,
    gray: &Arc<image::RgbaImage>,
    brightmaps: &Brightmaps,
    pbr: &PbrPack,
    min_x: i32,
    min_h: i32,
    max_y: i32,
    grid: i32,
    voxels: &mut HashMap<u64, Voxel>,
) {
    let right = side_sector(sides, line.right);
    let left = side_sector(sides, line.left);
    if let Some(front) = right {
        draw_side(
            line, true, front, left, sides, sectors, pictures, gray, brightmaps, pbr, min_x, min_h,
            max_y, grid, voxels,
        );
    }
    if let Some(front) = left {
        draw_side(
            line, false, front, right, sides, sectors, pictures, gray, brightmaps, pbr, min_x, min_h,
            max_y, grid, voxels,
        );
    }
}

fn draw_side(
    line: &Line,
    front_is_right: bool,
    front: usize,
    back: Option<usize>,
    sides: &[Side],
    sectors: &[Sector],
    pictures: &HashMap<String, Arc<image::RgbaImage>>,
    gray: &Arc<image::RgbaImage>,
    brightmaps: &Brightmaps,
    pbr: &PbrPack,
    min_x: i32,
    min_h: i32,
    max_y: i32,
    grid: i32,
    voxels: &mut HashMap<u64, Voxel>,
) {
    if front >= sectors.len() {
        return;
    }
    let side_index = if front_is_right {
        line.right
    } else {
        line.left
    };
    if side_index < 0 {
        return;
    }
    let Some(side) = sides.get(side_index as usize) else {
        return;
    };
    let (x0, y0, x1, y1) = if front_is_right {
        (line.x0, line.y0, line.x1, line.y1)
    } else {
        (line.x1, line.y1, line.x0, line.y0)
    };
    let light = sectors[front].light;
    let front_floor = sectors[front].floor as i32;
    let front_ceil = sectors[front].ceil as i32;
    let blocking = line.flags & 1 != 0;
    let unpeg_bottom = line.flags & 8 != 0;
    let unpeg_top = line.flags & 16 != 0;
    let cells = cells_on_line(x0, y0, x1, y1);
    let xoff = side.xoff;
    let yoff = side.yoff;
    let upper = side.upper.clone();
    let lower = side.lower.clone();
    let mid = side.mid.clone();

    let draw = |h0: i32,
                h1: i32,
                name: &str,
                tex: &image::RgbaImage,
                peg: Peg,
                voxels: &mut HashMap<u64, Voxel>| {
        paint_columns(
            h0,
            h1,
            tex,
            brightmaps.texture(name),
            peg,
            &cells,
            x0,
            y0,
            x1,
            y1,
            xoff,
            yoff,
            light,
            pbr.get(name),
            min_x,
            min_h,
            max_y,
            grid,
            voxels,
        );
    };

    if back.is_none() || back == Some(front) {
        let img = pictures
            .get(&mid)
            .map(|img| img.as_ref())
            .unwrap_or(gray.as_ref());
        let peg = if unpeg_bottom {
            Peg::Bottom(front_floor)
        } else {
            Peg::Top(front_ceil)
        };
        draw(front_floor, front_ceil, &mid, img, peg, voxels);
        return;
    }
    let back = back.unwrap();
    if back >= sectors.len() {
        return;
    }
    let back_floor = sectors[back].floor as i32;
    let back_ceil = sectors[back].ceil as i32;
    // Doom treats adjoining F_SKY1 ceilings as one continuous, infinitely distant
    // sky even when their sector heights differ. The upper wall is a sky portal,
    // not opaque geometry.
    let joined_sky = sectors[front].ceil_flat == "F_SKY1" && sectors[back].ceil_flat == "F_SKY1";
    if front_ceil > back_ceil && !joined_sky {
        if let Some(img) = pictures.get(&upper) {
            let peg = if unpeg_top {
                Peg::Top(front_ceil)
            } else {
                Peg::Bottom(back_ceil)
            };
            draw(back_ceil, front_ceil, &upper, img, peg, voxels);
        }
    }
    if front_floor < back_floor {
        if let Some(img) = pictures.get(&lower) {
            let peg = if unpeg_bottom {
                Peg::Bottom(front_floor)
            } else {
                Peg::Top(back_floor)
            };
            draw(front_floor, back_floor, &lower, img, peg, voxels);
        }
    }
    let opening_floor = front_floor.max(back_floor);
    let opening_ceil = front_ceil.min(back_ceil);
    if mid != "-" && !mid.is_empty() {
        if let Some(img) = pictures.get(&mid) {
            if blocking {
                let peg = if unpeg_bottom {
                    Peg::Bottom(opening_floor)
                } else {
                    Peg::Top(opening_ceil)
                };
                draw(opening_floor, opening_ceil, &mid, img, peg, voxels);
            }
        }
    }
}

fn paint_columns(
    h0: i32,
    h1: i32,
    tex: &image::RgbaImage,
    brightmap: Option<&image::RgbaImage>,
    peg: Peg,
    cells: &[(i32, i32)],
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    xoff: i32,
    yoff: i32,
    light: i16,
    maps: Option<&PbrTex>,
    min_x: i32,
    min_h: i32,
    max_y: i32,
    grid: i32,
    voxels: &mut HashMap<u64, Voxel>,
) {
    if h1 <= h0 {
        return;
    }
    let len2 = (x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0);
    let len = len2.sqrt();
    // The line is walked with the front sector on the right. That direction,
    // not the cube face the ray happens to enter, is the wall's shading normal.
    let (normal, tangent) = if len < 1.0e-4 {
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0])
    } else {
        let dx = (x1 - x0) / len;
        let dy = (y1 - y0) / len;
        let wad_nx = dy;
        let wad_ny = -dx;
        ([wad_nx, 0.0, -wad_ny], [dx, 0.0, -dy])
    };
    let bitangent = [0.0, 1.0, 0.0];
    for (cx, cy) in cells {
        let dist = if len2 < 1e-4 {
            0.0
        } else {
            let t =
                ((*cx as f32 + 0.5 - x0) * (x1 - x0) + (*cy as f32 + 0.5 - y0) * (y1 - y0)) / len2;
            t.clamp(0.0, 1.0) * len
        };
        let u = dist as i32 + xoff;
        for h in h0..h1 {
            let v = peg.row(h, tex.height() as i32, yoff);
            let pixel = sample_rgba(tex, u, v);
            if pixel[3] < 128 {
                continue;
            }
            let rgb = [pixel[0], pixel[1], pixel[2]];
            let emission = sample_emission(brightmap, tex, u, v);
            let (ao, sampled) = surface_of(tex, u, v, maps);
            let rgb = shade_rgb(rgb, ao);
            paint(
                voxels,
                grid,
                min_x,
                min_h,
                max_y,
                *cx,
                h,
                *cy,
                rgb,
                light,
                emission,
                orient(normal, tangent, bitangent, &sampled),
            );
        }
    }
}

#[derive(Clone, Copy)]
enum Peg {
    Top(i32),
    Bottom(i32),
}

impl Peg {
    fn row(self, height: i32, tex_h: i32, yoff: i32) -> i32 {
        match self {
            Peg::Top(top) => top - height + yoff,
            Peg::Bottom(bottom) => tex_h - (height - bottom) + yoff,
        }
    }
}

fn cells_on_line(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(i32, i32)> {
    let mut x = x0.floor() as i32;
    let mut y = y0.floor() as i32;
    let x_end = x1.floor() as i32;
    let y_end = y1.floor() as i32;
    let mut cells = vec![(x, y)];
    if x == x_end && y == y_end {
        return cells;
    }
    let dx = x1 - x0;
    let dy = y1 - y0;
    let step_x = if dx >= 0.0 { 1 } else { -1 };
    let step_y = if dy >= 0.0 { 1 } else { -1 };
    let inf = 1.0e9_f32;
    let tdx = if dx.abs() < 1e-8 {
        inf
    } else {
        (1.0 / dx).abs()
    };
    let tdy = if dy.abs() < 1e-8 {
        inf
    } else {
        (1.0 / dy).abs()
    };
    let mut tmax_x = if dx.abs() < 1e-8 {
        inf
    } else if dx > 0.0 {
        (x as f32 + 1.0 - x0) / dx
    } else {
        (x as f32 - x0) / dx
    };
    let mut tmax_y = if dy.abs() < 1e-8 {
        inf
    } else if dy > 0.0 {
        (y as f32 + 1.0 - y0) / dy
    } else {
        (y as f32 - y0) / dy
    };
    let limit = ((x0 - x1).abs() + (y0 - y1).abs()) as i32 + 8;
    for _ in 0..limit.max(1) {
        if (tmax_x - tmax_y).abs() < 1e-4 {
            cells.push((x + step_x, y));
            cells.push((x, y + step_y));
            tmax_x += tdx;
            tmax_y += tdy;
            x += step_x;
            y += step_y;
        } else if tmax_x < tmax_y {
            tmax_x += tdx;
            x += step_x;
        } else {
            tmax_y += tdy;
            y += step_y;
        }
        cells.push((x, y));
        if x == x_end && y == y_end {
            break;
        }
    }
    cells
}

fn sector_at(edges: &[Vec<Edge>], x: f32, y: f32) -> Option<usize> {
    edges.iter().position(|sector| inside(sector, x, y))
}

fn inside(edges: &[Edge], x: f32, y: f32) -> bool {
    let mut crossings = 0;
    for edge in edges {
        if (edge.y0 > y) != (edge.y1 > y) {
            let t = (y - edge.y0) / (edge.y1 - edge.y0);
            let xi = edge.x0 + t * (edge.x1 - edge.x0);
            if x < xi {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}

fn find_floor(edges: &[Vec<Edge>], sectors: &[Sector], x: f32, y: f32) -> Option<i16> {
    const NUDGE: [(f32, f32); 9] = [
        (0.0, 0.0),
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (8.0, 8.0),
        (-8.0, 8.0),
        (8.0, -8.0),
        (-8.0, -8.0),
    ];
    for (dx, dy) in NUDGE {
        if let Some(index) = sector_at(edges, x + dx, y + dy) {
            return Some(sectors[index].floor);
        }
    }
    None
}

fn sprite_bytes<'a>(
    data: &'a [u8],
    lumps: &HashMap<String, (usize, usize)>,
    name: &str,
    frame: u8,
) -> Option<&'a [u8]> {
    let frame = frame as char;
    for rot in ['0', '1'] {
        let key = format!("{name}{frame}{rot}");
        if let Some((off, size)) = lumps.get(&key) {
            return data.get(*off..*off + *size);
        }
    }
    None
}

fn is_marker(kind: i16) -> bool {
    matches!(kind, 1 | 2 | 3 | 4 | 11 | 14 | 87)
}

/// Doomednum to the spawn sprite and frame, from the Doom thing table.
fn sprite_of(kind: i16) -> Option<(&'static str, u8)> {
    const TABLE: &[(i16, &str, u8)] = &[
        (5, "BKEY", b'A'),
        (6, "YKEY", b'A'),
        (7, "SPID", b'A'),
        (8, "BPAK", b'A'),
        (9, "SPOS", b'A'),
        (10, "PLAY", b'W'),
        (12, "PLAY", b'W'),
        (13, "RKEY", b'A'),
        (15, "PLAY", b'N'),
        (16, "CYBR", b'A'),
        (17, "CELP", b'A'),
        (18, "POSS", b'L'),
        (19, "SPOS", b'L'),
        (20, "TROO", b'M'),
        (21, "SARG", b'N'),
        (22, "HEAD", b'L'),
        (23, "SKUL", b'K'),
        (24, "POL5", b'A'),
        (25, "POL1", b'A'),
        (26, "POL6", b'A'),
        (27, "POL4", b'A'),
        (28, "POL2", b'A'),
        (29, "POL3", b'A'),
        (30, "COL1", b'A'),
        (31, "COL2", b'A'),
        (32, "COL3", b'A'),
        (33, "COL4", b'A'),
        (34, "CAND", b'A'),
        (35, "CBRA", b'A'),
        (36, "COL5", b'A'),
        (37, "COL6", b'A'),
        (38, "RSKU", b'A'),
        (39, "YSKU", b'A'),
        (40, "BSKU", b'A'),
        (41, "CEYE", b'A'),
        (42, "FSKU", b'A'),
        (43, "TRE1", b'A'),
        (44, "TBLU", b'A'),
        (45, "TGRN", b'A'),
        (46, "TRED", b'A'),
        (47, "SMIT", b'A'),
        (48, "ELEC", b'A'),
        (49, "GOR1", b'A'),
        (50, "GOR2", b'A'),
        (51, "GOR3", b'A'),
        (52, "GOR4", b'A'),
        (53, "GOR5", b'A'),
        (54, "TRE2", b'A'),
        (55, "SMBT", b'A'),
        (56, "SMGT", b'A'),
        (57, "SMRT", b'A'),
        (58, "SARG", b'A'),
        (59, "GOR2", b'A'),
        (60, "GOR4", b'A'),
        (61, "GOR3", b'A'),
        (62, "GOR5", b'A'),
        (63, "GOR1", b'A'),
        (64, "VILE", b'A'),
        (65, "CPOS", b'A'),
        (66, "SKEL", b'A'),
        (67, "FATT", b'A'),
        (68, "BSPI", b'A'),
        (69, "BOS2", b'A'),
        (70, "FCAN", b'A'),
        (71, "PAIN", b'A'),
        (72, "KEEN", b'A'),
        (73, "HDB1", b'A'),
        (74, "HDB2", b'A'),
        (75, "HDB3", b'A'),
        (76, "HDB4", b'A'),
        (77, "HDB5", b'A'),
        (78, "HDB6", b'A'),
        (79, "POB1", b'A'),
        (80, "POB2", b'A'),
        (81, "BRS1", b'A'),
        (82, "SGN2", b'A'),
        (83, "MEGA", b'A'),
        (84, "SSWV", b'A'),
        (85, "TLMP", b'A'),
        (86, "TLP2", b'A'),
        (88, "BBRN", b'A'),
        (89, "SSWV", b'A'),
        (2001, "SHOT", b'A'),
        (2002, "MGUN", b'A'),
        (2003, "LAUN", b'A'),
        (2004, "PLAS", b'A'),
        (2005, "CSAW", b'A'),
        (2006, "BFUG", b'A'),
        (2007, "CLIP", b'A'),
        (2008, "SHEL", b'A'),
        (2010, "ROCK", b'A'),
        (2011, "STIM", b'A'),
        (2012, "MEDI", b'A'),
        (2013, "SOUL", b'A'),
        (2014, "BON1", b'A'),
        (2015, "BON2", b'A'),
        (2018, "ARM1", b'A'),
        (2019, "ARM2", b'A'),
        (2022, "PINV", b'A'),
        (2023, "PSTR", b'A'),
        (2024, "PINS", b'A'),
        (2025, "SUIT", b'A'),
        (2026, "PMAP", b'A'),
        (2028, "COLU", b'A'),
        (2035, "BAR1", b'A'),
        (2045, "PVIS", b'A'),
        (2046, "BROK", b'A'),
        (2047, "CELL", b'A'),
        (2048, "AMMO", b'A'),
        (2049, "SBOX", b'A'),
        (3001, "TROO", b'A'),
        (3002, "SARG", b'A'),
        (3003, "BOSS", b'A'),
        (3004, "POSS", b'A'),
        (3005, "HEAD", b'A'),
        (3006, "SKUL", b'A'),
    ];
    TABLE
        .binary_search_by_key(&kind, |row| row.0)
        .ok()
        .map(|i| (TABLE[i].1, TABLE[i].2))
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn zombieman_uses_the_possessed_sprite() {
        assert_eq!(super::sprite_of(3004), Some(("POSS", b'A')));
        assert_eq!(super::sprite_of(2035), Some(("BAR1", b'A')));
    }

    #[test]
    fn brightmap_samples_encode_mapped_dark_and_intensity() {
        let source = RgbaImage::new(2, 1);
        let mut map = RgbaImage::new(2, 1);
        map.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        map.put_pixel(1, 0, Rgba([255, 255, 255, 255]));

        assert_eq!(super::sample_emission(None, &source, 0, 0), 0);
        assert_eq!(super::sample_emission(Some(&map), &source, 0, 0), 1);
        assert_eq!(super::sample_emission(Some(&map), &source, 1, 0), 7);
    }

    #[test]
    fn pbr_maps_set_tilt_roughness_and_metal() {
        let mut normal = RgbaImage::new(2, 1);
        normal.put_pixel(0, 0, Rgba([255, 128, 255, 255]));
        normal.put_pixel(1, 0, Rgba([128, 128, 255, 255]));
        let mut rough = RgbaImage::new(2, 1);
        rough.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        rough.put_pixel(1, 0, Rgba([255, 255, 255, 255]));
        let mut metal = RgbaImage::new(2, 1);
        metal.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        metal.put_pixel(1, 0, Rgba([0, 0, 0, 255]));
        let maps = super::PbrTex {
            normal: Some(Arc::new(normal)),
            metallic: Some(Arc::new(metal)),
            roughness: Some(Arc::new(rough)),
            ao: None,
            glossiness: None,
        };
        let src = RgbaImage::new(2, 1);
        let (ao, bumped) = super::surface_of(&src, 0, 0, Some(&maps));
        assert_eq!(ao, 1.0);
        assert_eq!(bumped.metal, 1);
        assert_eq!(bumped.roughness, 1);
        let wall = super::orient([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], &bumped);
        let leaned = crate::octree::unpack_oct_normal(wall.normal).unwrap();
        assert!(leaned[0] > 0.4, "a bright red normal leans along the wall {:?}", leaned);
        assert!(leaned[2] > 0.4, "{:?}", leaned);
        let (_, flat) = super::surface_of(&src, 1, 0, Some(&maps));
        assert_eq!(flat.metal, 0);
        assert_eq!(flat.roughness, 3);
        let straight = super::orient([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], &flat);
        let n = crate::octree::unpack_oct_normal(straight.normal).unwrap();
        assert!(n[2] > 0.9 && n[0].abs() < 0.2, "{:?}", n);
    }

    #[test]
    fn gldefs_last_material_wins_and_ignores_comments() {
        let defs = super::parse_gldefs(
            r#"
            material texture "DOOR3" {
                normal "materials/normalmaps/auto/door3_n.dds"
                roughness "materials/roughness/auto/door3_r.dds"
            }
            material texture "DOOR3" {
                // normal "materials/normalmaps/auto/door3_n.dds"
                glossiness 22
            }
            "#,
        );
        let door = defs.get("DOOR3").unwrap();
        assert!(door.normal.is_none());
        assert!(door.roughness.is_none());
        assert_eq!(door.glossiness, Some(22.0));
    }

    #[test]
    fn dds_bgra_swaps_to_rgba() {
        let mut bytes = vec![0u8; 128 + 8];
        bytes[..4].copy_from_slice(b"DDS ");
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&2u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&8u32.to_le_bytes());
        bytes[88..92].copy_from_slice(&32u32.to_le_bytes());
        bytes[128..132].copy_from_slice(&[1, 2, 3, 4]);
        bytes[132..136].copy_from_slice(&[5, 6, 7, 8]);
        let image = super::decode_dds(&bytes).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [3, 2, 1, 4]);
        assert_eq!(image.get_pixel(1, 0).0, [7, 6, 5, 8]);
    }

    #[test]
    fn demean_tangent_removes_a_constant_lean() {
        let mut image = RgbaImage::new(2, 1);
        image.put_pixel(0, 0, Rgba([40, 200, 255, 255]));
        image.put_pixel(1, 0, Rgba([80, 220, 255, 255]));
        let centered = super::demean_tangent(&image);
        assert_eq!(centered.get_pixel(0, 0).0[0], 108);
        assert_eq!(centered.get_pixel(1, 0).0[0], 148);
        assert_eq!(centered.get_pixel(0, 0).0[2], 255);
    }

    #[test]
    fn dxt1_block_decodes_endpoint_color() {
        let mut bytes = vec![0u8; 128 + 8];
        bytes[..4].copy_from_slice(b"DDS ");
        bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&4u32.to_le_bytes());
        bytes[84..88].copy_from_slice(b"DXT1");
        // c0 white > c1 black, every index 0, so the block is the first endpoint.
        bytes[128..130].copy_from_slice(&0xffffu16.to_le_bytes());
        let image = super::decode_dds(&bytes).unwrap();
        assert_eq!(image.dimensions(), (4, 4));
        assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(3, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn transparent_wall_texels_do_not_create_voxels() {
        let transparent = RgbaImage::new(1, 1);
        let mut voxels = HashMap::new();
        super::paint_columns(
            0,
            1,
            &transparent,
            None,
            super::Peg::Top(1),
            &[(0, 0)],
            0.0,
            0.0,
            1.0,
            0.0,
            0,
            0,
            255,
            None,
            0,
            0,
            0,
            2,
            &mut voxels,
        );
        assert!(voxels.is_empty());
    }

    #[test]
    fn brightmap_definitions_apply_aliases_and_ignore_comments() {
        let image = Arc::new(RgbaImage::new(1, 1));
        let mut maps = HashMap::from([("66".to_owned(), image.clone())]);
        let definitions = r#"
            brightmap texture compblue { map "brightmaps/textures/doom/66.png" iwad }
            // brightmap texture disabled { map "brightmaps/textures/doom/66.png" iwad }
        "#;

        super::apply_brightmap_definitions(&mut maps, definitions, "texture");

        assert!(Arc::ptr_eq(maps.get("COMPBLUE").unwrap(), &image));
        assert!(!maps.contains_key("DISABLED"));
    }

    #[test]
    fn local_brightmap_pack_loads_when_available() {
        if super::find_brightmap_dir().is_none() {
            return;
        }
        let maps = super::Brightmaps::load();
        assert!(maps.texture("COMPBLUE").is_some());
        assert!(maps.flat("GATE2").is_some());
    }
}
