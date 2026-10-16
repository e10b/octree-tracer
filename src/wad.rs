//! E1M1 from a Doom IWAD on disk. The commercial WAD is not part of this repo.
//! Patches and flats come from PLAYPAL. A lump whose bytes are a PNG is used as-is,
//! the way GZDoom accepts PNG textures.

use super::*;
use std::collections::HashMap;
use std::sync::Arc;

pub struct BuiltMap {
    pub voxels: HashMap<u64, Voxel>,
    pub spawn: [i32; 3],
    pub look: [f32; 3],
    pub grid: i32,
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

fn shade(rgb: [u8; 3], light: i16, emission: u8) -> Voxel {
    let factor = (light as f32 / 255.0).clamp(0.25, 1.0);
    let ch = |c: u8| {
        let v = (c as f32 * factor).round() as u8;
        if v == 0 {
            1
        } else {
            v
        }
    };
    Voxel::with_emission(ch(rgb[0]), ch(rgb[1]), ch(rgb[2]), emission)
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
) {
    let x = wad_x - min_x;
    let y = height - min_h;
    let z = max_y - wad_y;
    if x < 0 || y < 0 || z < 0 || x >= grid || y >= grid || z >= grid {
        return;
    }
    voxels.insert(pack(x, y, z), shade(rgb, light, emission));
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
                    let rgb = sample(floor_img, x, y);
                    let emission = sample_emission(floor_brightmap, floor_img, x, y);
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
                    );
                    if !sky {
                        if let Some(ceil) = ceil_img {
                            let rgb = sample(ceil, x, y);
                            let emission = sample_emission(ceil_brightmap, ceil, x, y);
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
            line, true, front, left, sides, sectors, pictures, gray, brightmaps, min_x, min_h,
            max_y, grid, voxels,
        );
    }
    if let Some(front) = left {
        draw_side(
            line, false, front, right, sides, sectors, pictures, gray, brightmaps, min_x, min_h,
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
            paint(
                voxels, grid, min_x, min_h, max_y, *cx, h, *cy, rgb, light, emission,
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
