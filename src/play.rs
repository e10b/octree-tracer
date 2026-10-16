use super::*;
use std::collections::HashMap;

const STILL: u8 = 0;
const IDLE: u8 = 1;
const CHASE: u8 = 2;
const ATTACK: u8 = 3;
const PAIN: u8 = 4;
const DEATH: u8 = 5;
const BURST: u8 = 6;

const PROP: u8 = 0;
const BARREL: u8 = 1;
const ZOMBIE: u8 = 2;
const SG: u8 = 3;
const IMP: u8 = 4;
const DEMON: u8 = 5;

/// A thing from the map, drawn as one camera-facing plane.
pub struct Plane {
    /// World position of the thing's feet, on the floor.
    pub feet: [f32; 3],
    pub kind: i16,
    pub taken: bool,
    pub sprite: &'static str,
    pub frame: u8,
    pub health: i16,
    pub radius: f32,
    pub tall: f32,
    pub speed: f32,
    pub mode: u8,
    pub mode_t: f32,
    pub next_act: f32,
    pub see_t: f32,
    pub awake: bool,
    pub acted: bool,
    pub swing: u8,
}

pub struct Clip {
    pub uv: [f32; 4],
    pub px_w: f32,
    pub px_h: f32,
    pub left: i32,
    pub top: i32,
}

pub struct GunSprite {
    pub label: &'static str,
    /// Atlas clips, in frame order A, B, C...
    pub frames: Vec<usize>,
    pub flash: Vec<usize>,
}

pub struct Pictures {
    pub atlas: Vec<u8>,
    pub atlas_size: u32,
    pub planes: Vec<Plane>,
    pub guns: Vec<GunSprite>,
    pub clips: Vec<Clip>,
    pub names: HashMap<String, usize>,
    pub doors: Vec<Door>,
    /// World size of one map unit.
    pub voxel: f32,
}

/// A door slab. It lives in the linedef and slides up, so the octree doorway stays air.
pub struct Door {
    pub sector: u16,
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub height: f32,
    pub lift: f32,
    motion: DoorMotion,
    phase_t: f32,
    pub uv: [f32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DoorMotion {
    Shut,
    Opening,
    Waiting,
    Closing,
}

impl Pictures {
    pub fn empty() -> Self {
        Self {
            atlas: vec![0, 0, 0, 0],
            atlas_size: 1,
            planes: Vec::new(),
            guns: Vec::new(),
            clips: Vec::new(),
            names: HashMap::new(),
            doors: Vec::new(),
            voxel: 1.0,
        }
    }
}

pub struct PendingPlane {
    pub kind: i16,
    pub feet: [f32; 3],
    pub sprite: &'static str,
    pub frame: u8,
}

pub struct PendingClip {
    pub name: String,
    pub image: image::RgbaImage,
    pub left: i32,
    pub top: i32,
}

pub struct PendingGun {
    pub label: &'static str,
    pub frames: Vec<String>,
    pub flash: Vec<String>,
}

pub struct PendingDoor {
    pub sector: u16,
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub height: f32,
    pub image: image::RgbaImage,
}

pub fn pack(
    planes: Vec<PendingPlane>,
    clips: Vec<PendingClip>,
    guns: Vec<PendingGun>,
    doors: Vec<PendingDoor>,
    voxel: f32,
) -> Pictures {
    const SIZE: u32 = 2048;
    let mut shelf = Shelf::new(SIZE);
    let mut out_clips = Vec::with_capacity(clips.len());
    let mut names = HashMap::new();
    for clip in clips {
        let w = clip.image.width().max(1) as f32;
        let h = clip.image.height().max(1) as f32;
        let uv = shelf.place(&clip.image);
        names.insert(clip.name, out_clips.len());
        out_clips.push(Clip {
            uv,
            px_w: w,
            px_h: h,
            left: clip.left,
            top: clip.top,
        });
    }
    let mut out_guns = Vec::with_capacity(guns.len());
    for gun in guns {
        let frames = gun
            .frames
            .iter()
            .filter_map(|name| names.get(name).copied())
            .collect();
        let flash = gun
            .flash
            .iter()
            .filter_map(|name| names.get(name).copied())
            .collect();
        out_guns.push(GunSprite {
            label: gun.label,
            frames,
            flash,
        });
    }
    let mut out_doors = Vec::with_capacity(doors.len());
    for door in doors {
        let uv = shelf.place(&door.image);
        out_doors.push(Door {
            sector: door.sector,
            a: door.a,
            b: door.b,
            height: door.height,
            lift: 0.0,
            motion: DoorMotion::Shut,
            phase_t: 0.0,
            uv,
        });
    }
    let planes = planes.into_iter().map(finish_plane).collect();
    Pictures {
        atlas: shelf.atlas.into_raw(),
        atlas_size: SIZE,
        planes,
        guns: out_guns,
        clips: out_clips,
        names,
        doors: out_doors,
        voxel,
    }
}

fn finish_plane(plane: PendingPlane) -> Plane {
    let (health, radius, tall, speed, _) = kind_stats(plane.kind);
    let alive = health > 0;
    Plane {
        feet: plane.feet,
        kind: plane.kind,
        taken: false,
        sprite: plane.sprite,
        frame: plane.frame,
        health,
        radius,
        tall,
        speed,
        mode: if alive { IDLE } else { STILL },
        mode_t: 0.0,
        next_act: 0.4,
        see_t: (plane.kind as u32 % 8) as f32 * 0.04,
        awake: false,
        acted: false,
        swing: 0,
    }
}

struct Shelf {
    size: u32,
    atlas: image::RgbaImage,
    x: u32,
    y: u32,
    row_h: u32,
}

impl Shelf {
    fn new(size: u32) -> Self {
        Self {
            size,
            atlas: image::RgbaImage::new(size, size),
            x: 1,
            y: 1,
            row_h: 1,
        }
    }

    fn place(&mut self, img: &image::RgbaImage) -> [f32; 4] {
        let w = img.width();
        let h = img.height();
        if w == 0 || h == 0 || w + 2 > self.size || h + 2 > self.size {
            return [0.0, 0.0, 0.0, 0.0];
        }
        if self.x + w + 1 > self.size {
            self.x = 1;
            self.y += self.row_h + 1;
            self.row_h = 1;
        }
        if self.y + h + 1 > self.size {
            return [0.0, 0.0, 0.0, 0.0];
        }
        for py in 0..h {
            for px in 0..w {
                self.atlas
                    .put_pixel(self.x + px, self.y + py, *img.get_pixel(px, py));
            }
        }
        let s = self.size as f32;
        let uv = [
            self.x as f32 / s,
            self.y as f32 / s,
            (self.x + w) as f32 / s,
            (self.y + h) as f32 / s,
        ];
        self.x += w + 1;
        self.row_h = self.row_h.max(h);
        uv
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteVert {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub dist: f32,
    pub _pad: f32,
}

pub struct Play {
    pub pictures: Pictures,
    pub weapon: usize,
    pub health: i32,
    pub armor: i32,
    pub ammo: i32,
    pub keys: String,
    pub note: String,
    pub note_until: f64,
    pub kick: f32,
    pub gun_busy: bool,
    pub gun_step: usize,
    pub gun_left: f32,
    pub flash_left: f32,
    flash_for: f32,
    pub shots: u32,
    rocket_aim: Option<(Point3<f32>, Vector3<f32>)>,
    rocket_grid: i32,
    rng: u32,
    flyers: Vec<Flyer>,
}

impl Play {
    pub fn new(pictures: Pictures) -> Self {
        Self {
            pictures,
            weapon: 0,
            health: 100,
            armor: 0,
            ammo: 50,
            keys: String::new(),
            note: String::new(),
            note_until: 0.0,
            kick: 0.0,
            gun_busy: false,
            gun_step: 0,
            gun_left: 0.0,
            flash_left: 0.0,
            flash_for: 0.0,
            shots: 0,
            rocket_aim: None,
            rocket_grid: 0,
            rng: 1,
            flyers: Vec::new(),
        }
    }

    pub fn fire(
        &mut self,
        _time: f64,
        eye: Point3<f32>,
        look: Vector3<f32>,
        octree: &CpuOctree,
        grid: i32,
    ) {
        if self.ammo <= 0 || self.gun_busy {
            return;
        }
        let dir = look.normalize();
        let label = self.weapon_label().to_string();
        let seq = gun_sequence(&label);
        self.ammo -= 1;
        self.shots = self.shots.wrapping_add(1);
        self.gun_busy = true;
        self.gun_step = 0;
        self.gun_left = seq.hold[0];
        self.flash_for = seq.flash_hold;
        self.flash_left = seq.flash_hold;
        self.kick = 0.0;
        if label == "ROCKET" {
            self.rocket_aim = Some((eye, dir));
            self.rocket_grid = grid;
            return;
        }
        let pellets = if label == "SHOTGUN" { 7 } else { 1 };
        for pellet in 0..pellets {
            let aim = if pellets == 1 {
                dir
            } else {
                spread(dir, pellet)
            };
            self.shoot_ray(eye, aim, octree, grid);
        }
    }

    pub fn tick(&mut self, dt: f32, eye: Point3<f32>, octree: &CpuOctree, grid: i32) -> i32 {
        self.advance_gun(dt);
        self.tick_doors(dt, eye, grid);
        let mut pain = 0;
        let n = self.pictures.planes.len();
        for index in 0..n {
            pain += self.tick_actor(index, dt, eye, octree, grid);
        }
        pain += self.tick_flyers(dt, eye, octree, grid);
        pain
    }

    pub fn hurt_player(&mut self, dmg: i32) {
        if dmg <= 0 || self.health <= 0 {
            return;
        }
        let mut rest = dmg;
        if self.armor > 0 {
            let saved = (dmg / 3).min(self.armor);
            self.armor -= saved;
            rest -= saved;
        }
        self.health = (self.health - rest).max(0);
        if self.health == 0 {
            self.note = "Dead".to_string();
            self.note_until = 1.0e9;
        }
    }

    /// Apply damage to one actor. Used by shots, barrels, and rockets.
    pub fn strike(
        &mut self,
        index: usize,
        dmg: i32,
        eye: Point3<f32>,
        octree: &CpuOctree,
        grid: i32,
    ) -> i32 {
        self.apply_damage(index, dmg, eye, octree, grid)
    }

    fn weapon_label(&self) -> &str {
        self.pictures
            .guns
            .get(self.weapon)
            .map(|gun| gun.label)
            .unwrap_or("PISTOL")
    }

    pub fn take_pickups(&mut self, pos: Point3<f32>, grid: i32, time: f64) {
        let voxel = 2.0 / grid.max(1) as f32;
        let reach = 24.0 * voxel;
        let tall = 96.0 * voxel;
        let mut hit = Vec::new();
        for (index, plane) in self.pictures.planes.iter().enumerate() {
            if plane.taken || !is_item(plane.kind) {
                continue;
            }
            let dx = pos.x - plane.feet[0];
            let dy = pos.y - plane.feet[1];
            let dz = pos.z - plane.feet[2];
            if dx * dx + dz * dz <= reach * reach && dy.abs() <= tall {
                hit.push(index);
            }
        }
        for index in hit {
            let kind = self.pictures.planes[index].kind;
            let note = self.apply(kind);
            if note.is_empty() {
                continue;
            }
            self.pictures.planes[index].taken = true;
            self.note = note.to_string();
            self.note_until = time + 1.6;
        }
    }

    fn apply(&mut self, kind: i16) -> &'static str {
        match kind {
            2011 => {
                self.health = (self.health + 10).min(100);
                "Stimpack"
            }
            2012 => {
                self.health = (self.health + 25).min(100);
                "Medikit"
            }
            2014 => {
                self.health = (self.health + 1).min(200);
                "Health bonus"
            }
            2015 => {
                self.armor = (self.armor + 1).min(200);
                "Armor bonus"
            }
            2018 => {
                self.armor = self.armor.max(100);
                "Green armor"
            }
            2019 => {
                self.armor = self.armor.max(200);
                "Blue armor"
            }
            2007 => {
                self.ammo += 10;
                "Clip"
            }
            2008 => {
                self.ammo += 4;
                "Shells"
            }
            2048 => {
                self.ammo += 50;
                "Box of bullets"
            }
            2049 => {
                self.ammo += 20;
                "Box of shells"
            }
            2046 => {
                self.ammo += 5;
                "Box of rockets"
            }
            2013 => {
                self.health = (self.health + 100).min(200);
                "Soulsphere"
            }
            8 => {
                self.ammo += 10;
                "Backpack"
            }
            2001 => {
                self.equip("SHOTGUN");
                self.ammo += 8;
                "Shotgun"
            }
            2002 => {
                self.equip("CHAINGUN");
                self.ammo += 20;
                "Chaingun"
            }
            2003 => {
                self.equip("ROCKET");
                self.ammo += 2;
                "Rocket launcher"
            }
            5 | 40 => {
                self.add_key('B');
                "Blue key"
            }
            6 | 39 => {
                self.add_key('Y');
                "Yellow key"
            }
            13 | 38 => {
                self.add_key('R');
                "Red key"
            }
            _ => "",
        }
    }

    fn equip(&mut self, label: &str) {
        if let Some(index) = self.pictures.guns.iter().position(|gun| gun.label == label) {
            self.weapon = index;
        }
    }

    fn add_key(&mut self, key: char) {
        if !self.keys.contains(key) {
            if !self.keys.is_empty() {
                self.keys.push(' ');
            }
            self.keys.push(key);
        }
    }

    /// E, in front of a shut or closing door, starts it rising. A 128-unit door
    /// takes about half a second, waits, then comes back down.
    pub fn use_facing(&mut self, eye: Point3<f32>, look: Vector3<f32>, _grid: i32) {
        let mut flat = Vector3::new(look.x, 0.0, look.z);
        if flat.magnitude2() < 1.0e-8 {
            return;
        }
        flat = flat.normalize();
        let reach = 64.0 * self.pictures.voxel;
        let mut best: Option<(u16, f32)> = None;
        for door in &self.pictures.doors {
            if door.motion == DoorMotion::Opening || door.motion == DoorMotion::Waiting {
                continue;
            }
            let (dist, cx, cz) = closest_xz(eye.x, eye.z, door);
            if dist > reach {
                continue;
            }
            let facing = if dist < 1.0e-4 {
                1.0
            } else {
                flat.x * (cx - eye.x) + flat.z * (cz - eye.z)
            };
            if facing <= 0.0 {
                continue;
            }
            if best.map(|(_, d)| dist < d).unwrap_or(true) {
                best = Some((door.sector, dist));
            }
        }
        let Some((sector, _)) = best else {
            return;
        };
        for door in &mut self.pictures.doors {
            if door.sector == sector {
                door.motion = DoorMotion::Opening;
            }
        }
    }

    pub fn door_blocks(&self, eye: Point3<f32>, grid: i32) -> bool {
        let voxel = 2.0 / grid as f32;
        let feet = eye.y - 41.0 * voxel;
        let head = feet + 56.0 * voxel;
        let radius = 16.0 * voxel;
        self.pictures.doors.iter().any(|door| {
            let bottom = door.a[1] + door.lift * door.height;
            let top = bottom + door.height;
            if head <= bottom + voxel || feet >= top - voxel {
                return false;
            }
            segment_dist_xz(eye.x, eye.z, door) < radius
        })
    }

    fn tick_doors(&mut self, dt: f32, eye: Point3<f32>, grid: i32) {
        let n = self.pictures.doors.len();
        for i in 0..n {
            let span = self.pictures.doors[i].height / self.pictures.voxel.max(1.0e-8);
            let rate = 280.0 / span.max(1.0);
            match self.pictures.doors[i].motion {
                DoorMotion::Shut => {}
                DoorMotion::Opening => {
                    self.pictures.doors[i].lift =
                        (self.pictures.doors[i].lift + rate * dt).min(1.0);
                    if self.pictures.doors[i].lift >= 1.0 {
                        self.pictures.doors[i].motion = DoorMotion::Waiting;
                        self.pictures.doors[i].phase_t = 0.0;
                    }
                }
                DoorMotion::Waiting => {
                    self.pictures.doors[i].phase_t += dt;
                    if self.pictures.doors[i].phase_t >= 4.3 && !self.doorway_busy(i, eye, grid) {
                        self.pictures.doors[i].motion = DoorMotion::Closing;
                    }
                }
                DoorMotion::Closing => {
                    if self.doorway_busy(i, eye, grid) {
                        self.pictures.doors[i].motion = DoorMotion::Opening;
                    } else {
                        self.pictures.doors[i].lift =
                            (self.pictures.doors[i].lift - rate * dt).max(0.0);
                        if self.pictures.doors[i].lift <= 0.0 {
                            self.pictures.doors[i].motion = DoorMotion::Shut;
                        }
                    }
                }
            }
        }
    }

    fn doorway_busy(&self, index: usize, eye: Point3<f32>, _grid: i32) -> bool {
        let door = &self.pictures.doors[index];
        let voxel = self.pictures.voxel;
        if segment_dist_xz(eye.x, eye.z, door) < 24.0 * voxel {
            return true;
        }
        self.pictures.planes.iter().any(|plane| {
            !plane.taken
                && plane.radius > 0.0
                && segment_dist_xz(plane.feet[0], plane.feet[2], door) < plane.radius * voxel
        })
    }

    pub fn quads(
        &self,
        eye: Point3<f32>,
        look: Vector3<f32>,
        fov: f32,
        screen_w: f32,
        screen_h: f32,
        time: f32,
        moving: bool,
    ) -> (Vec<SpriteVert>, Vec<SpriteVert>) {
        let mut flat = Vector3::new(look.x, 0.0, look.z);
        if flat.magnitude2() < 1.0e-8 {
            flat = Vector3::new(0.0, 0.0, 1.0);
        }
        flat = flat.normalize();
        let right = Vector3::unit_y().cross(flat);
        let view = Matrix4::look_at_rh(eye, eye + look, Vector3::unit_y());
        let aspect = if screen_w > 1.0 {
            screen_h / screen_w
        } else {
            1.0
        };

        let mut world = Vec::new();
        let voxel = self.pictures.voxel;
        for plane in &self.pictures.planes {
            if plane.taken {
                continue;
            }
            self.push_named(
                &mut world,
                eye,
                &view,
                fov,
                aspect,
                right,
                plane.feet,
                plane.sprite,
                plane.frame,
                voxel,
            );
        }
        for flyer in &self.flyers {
            self.push_named(
                &mut world,
                eye,
                &view,
                fov,
                aspect,
                right,
                flyer.pos,
                flyer.sprite,
                flyer.frame,
                voxel,
            );
        }
        for door in &self.pictures.doors {
            push_door(&mut world, eye, &view, fov, aspect, door);
        }

        let mut gun_verts = Vec::new();
        if screen_w > 1.0 && screen_h > 1.0 {
            let mut bob_x = 0.0;
            let mut bob_y = 0.0;
            if moving && !self.gun_busy {
                bob_x = (time * 9.0).sin() * 0.012;
                bob_y = (time * 18.0).sin().abs() * 0.01;
            }
            if let Some(index) = self.gun_clip() {
                let clip = &self.pictures.clips[index];
                push_view_sprite(&mut gun_verts, clip, screen_w, screen_h, bob_x, bob_y);
            }
            if self.flash_left > 0.0 {
                if let Some(index) = self.flash_clip() {
                    let clip = &self.pictures.clips[index];
                    push_view_sprite(&mut gun_verts, clip, screen_w, screen_h, bob_x, bob_y);
                }
            }
        }
        (world, gun_verts)
    }

    fn push_named(
        &self,
        world: &mut Vec<SpriteVert>,
        eye: Point3<f32>,
        view: &Matrix4<f32>,
        fov: f32,
        aspect: f32,
        right: Vector3<f32>,
        origin: [f32; 3],
        sprite: &str,
        frame: u8,
        voxel: f32,
    ) {
        let key = format!("{sprite}{}", frame as char);
        let Some(id) = self.pictures.names.get(&key) else {
            return;
        };
        let clip = &self.pictures.clips[*id];
        let sink = (clip.px_h - clip.top as f32).max(0.0) * voxel;
        let shift = (clip.px_w * 0.5 - clip.left as f32) * voxel;
        let feet = Vector3::new(
            origin[0] + right.x * shift,
            origin[1] - sink,
            origin[2] + right.z * shift,
        );
        let half = right * (clip.px_w * voxel * 0.5);
        let up = Vector3::unit_y() * (clip.px_h * voxel);
        let corners = [feet - half, feet + half, feet - half + up, feet + half + up];
        let mut ndc = [[0.0; 2]; 4];
        let mut dist = [0.0; 4];
        let origin_vp = view
            * Point3::new(origin[0], origin[1] + clip.px_h * voxel * 0.5, origin[2])
                .to_homogeneous();
        if -origin_vp.z < 0.02 {
            return;
        }
        for (i, corner) in corners.iter().enumerate() {
            let point = Point3::new(corner.x, corner.y, corner.z);
            let mut vp = view * point.to_homogeneous();
            if -vp.z < 0.05 {
                vp.z = origin_vp.z;
            }
            ndc[i] = project_ndc(vp, fov, aspect);
            dist[i] = (point - eye).magnitude().max(0.05);
        }
        push_quad(world, ndc, dist, clip.uv);
    }

    fn gun_clip(&self) -> Option<usize> {
        let gun = self.pictures.guns.get(self.weapon)?;
        if gun.frames.is_empty() {
            return None;
        }
        let seq = gun_sequence(gun.label);
        let slot = if gun.label == "CHAINGUN" {
            if self.gun_busy {
                self.shots.wrapping_sub(1) as usize % gun.frames.len()
            } else {
                0
            }
        } else if self.gun_busy {
            let step = self.gun_step.min(seq.frames.len() - 1);
            seq.frames[step] as usize
        } else {
            0
        };
        Some(gun.frames[slot.min(gun.frames.len() - 1)])
    }

    fn flash_clip(&self) -> Option<usize> {
        let gun = self.pictures.guns.get(self.weapon)?;
        if gun.flash.is_empty() {
            return None;
        }
        let slot = if gun.label == "CHAINGUN" {
            self.shots.wrapping_sub(1) as usize % gun.flash.len()
        } else if self.flash_for > 0.0 {
            let elapsed = (self.flash_for - self.flash_left).max(0.0);
            ((elapsed / self.flash_for) * gun.flash.len() as f32) as usize
        } else {
            0
        };
        Some(gun.flash[slot.min(gun.flash.len() - 1)])
    }

    pub fn hud(&self, ctx: &egui::CtxRef, time: f64) {
        let health_colour = if self.health > 50 {
            egui::Color32::from_rgb(80, 220, 80)
        } else if self.health > 25 {
            egui::Color32::from_rgb(230, 190, 40)
        } else {
            egui::Color32::from_rgb(220, 50, 40)
        };
        let weapon = self
            .pictures
            .guns
            .get(self.weapon)
            .map(|gun| gun.label)
            .unwrap_or("HANDS");
        egui::TopBottomPanel::bottom("hud")
            .frame(egui::Frame {
                margin: egui::Vec2::new(18.0, 8.0),
                corner_radius: 0.0,
                shadow: Default::default(),
                fill: egui::Color32::from_rgb(12, 10, 8),
                stroke: egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 70, 40)),
            })
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("HEALTH {}", self.health))
                            .heading()
                            .color(health_colour),
                    );
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new(format!("ARMOR {}", self.armor))
                            .heading()
                            .color(egui::Color32::from_rgb(80, 160, 230)),
                    );
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new(format!("AMMO {}", self.ammo))
                            .heading()
                            .color(egui::Color32::from_rgb(230, 200, 80)),
                    );
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new(weapon)
                            .heading()
                            .color(egui::Color32::from_rgb(230, 220, 200)),
                    );
                    if !self.keys.is_empty() {
                        ui.add_space(24.0);
                        ui.label(
                            egui::RichText::new(format!("KEYS {}", self.keys))
                                .heading()
                                .color(egui::Color32::from_rgb(200, 200, 210)),
                        );
                    }
                    if time < self.note_until && !self.note.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(), |ui| {
                            ui.label(
                                egui::RichText::new(&self.note)
                                    .heading()
                                    .color(egui::Color32::from_rgb(240, 230, 180)),
                            );
                        });
                    }
                });
            });

        egui::Area::new("crosshair")
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("+")
                        .heading()
                        .color(egui::Color32::WHITE),
                );
            });
    }

    fn advance_gun(&mut self, dt: f32) {
        if self.flash_left > 0.0 {
            self.flash_left = (self.flash_left - dt).max(0.0);
        }
        if !self.gun_busy {
            return;
        }
        self.gun_left -= dt;
        if self.gun_left > 0.0 {
            return;
        }
        let label = self.weapon_label().to_string();
        let seq = gun_sequence(&label);
        self.gun_step += 1;
        if label == "ROCKET" && self.gun_step == 1 {
            if let Some((eye, dir)) = self.rocket_aim.take() {
                let grid = self.rocket_grid;
                self.spawn_flyer(eye, dir, grid, usize::MAX, ROCKET_FLY, 700.0);
            }
        }
        if self.gun_step >= seq.hold.len() {
            self.gun_busy = false;
            self.gun_step = 0;
            self.gun_left = 0.0;
            return;
        }
        self.gun_left = seq.hold[self.gun_step];
    }

    fn shoot_ray(&mut self, eye: Point3<f32>, dir: Vector3<f32>, octree: &CpuOctree, grid: i32) {
        let wall = wall_distance(octree, grid, eye, dir);
        let voxel = 2.0 / grid as f32;
        let mut best: Option<(usize, f32)> = None;
        for (index, plane) in self.pictures.planes.iter().enumerate() {
            if !shootable(plane) {
                continue;
            }
            let Some(t) = hit_cylinder(
                eye,
                dir,
                plane.feet,
                plane.radius * voxel,
                plane.tall * voxel,
            ) else {
                continue;
            };
            if t >= wall {
                continue;
            }
            if best.map(|(_, bt)| t < bt).unwrap_or(true) {
                best = Some((index, t));
            }
        }
        if let Some((index, _)) = best {
            let dmg = (self.roll(3) + 1) * 4;
            self.apply_damage(index, dmg as i32, eye, octree, grid);
            return;
        }
        if wall < 200.0 {
            let t = (wall - 4.0 * voxel).max(voxel);
            self.puff(
                [eye.x + dir.x * t, eye.y + dir.y * t, eye.z + dir.z * t],
                "PUFF",
            );
        }
    }

    fn apply_damage(
        &mut self,
        index: usize,
        dmg: i32,
        eye: Point3<f32>,
        octree: &CpuOctree,
        grid: i32,
    ) -> i32 {
        if index >= self.pictures.planes.len() || dmg <= 0 {
            return 0;
        }
        let mode = self.pictures.planes[index].mode;
        if self.pictures.planes[index].health <= 0 || mode == DEATH || mode == BURST {
            return 0;
        }
        self.pictures.planes[index].health -= dmg as i16;
        self.pictures.planes[index].awake = true;
        if self.pictures.planes[index].health <= 0 {
            return self.kill_actor(index, eye, octree, grid);
        }
        let style = actor_style(self.pictures.planes[index].kind);
        if style != BARREL && style != PROP && self.roll(2) == 0 {
            self.pictures.planes[index].mode = PAIN;
            self.pictures.planes[index].mode_t = 0.0;
        } else if self.pictures.planes[index].mode == IDLE {
            self.pictures.planes[index].mode = CHASE;
        }
        0
    }

    fn kill_actor(&mut self, index: usize, eye: Point3<f32>, octree: &CpuOctree, grid: i32) -> i32 {
        let style = actor_style(self.pictures.planes[index].kind);
        self.pictures.planes[index].health = 0;
        self.pictures.planes[index].mode_t = 0.0;
        self.pictures.planes[index].acted = false;
        if style == BARREL {
            self.pictures.planes[index].mode = BURST;
            self.pictures.planes[index].sprite = "BEXP";
            self.pictures.planes[index].frame = b'A';
            let origin = self.pictures.planes[index].feet;
            return self.blast_at(origin, Some(index), eye, octree, grid);
        }
        self.pictures.planes[index].mode = DEATH;
        self.pictures.planes[index].awake = false;
        0
    }

    fn blast_at(
        &mut self,
        origin: [f32; 3],
        ignore: Option<usize>,
        eye: Point3<f32>,
        octree: &CpuOctree,
        grid: i32,
    ) -> i32 {
        let scale = grid as f32 * 0.5;
        let pdx = (eye.x - origin[0]) * scale;
        let pdz = (eye.z - origin[2]) * scale;
        let pdist = (pdx * pdx + pdz * pdz).sqrt();
        let mut pain = if pdist < 128.0 {
            (128.0 - pdist) as i32
        } else {
            0
        };
        let victims: Vec<(usize, i32)> = self
            .pictures
            .planes
            .iter()
            .enumerate()
            .filter_map(|(i, plane)| {
                if Some(i) == ignore || !shootable(plane) {
                    return None;
                }
                let dx = (plane.feet[0] - origin[0]) * scale;
                let dz = (plane.feet[2] - origin[2]) * scale;
                let dist = (dx * dx + dz * dz).sqrt();
                let dmg = 128.0 - dist;
                if dmg <= 0.0 {
                    None
                } else {
                    Some((i, dmg as i32))
                }
            })
            .collect();
        for (index, dmg) in victims {
            pain += self.apply_damage(index, dmg, eye, octree, grid);
        }
        pain
    }

    fn tick_actor(
        &mut self,
        index: usize,
        dt: f32,
        eye: Point3<f32>,
        octree: &CpuOctree,
        grid: i32,
    ) -> i32 {
        if self.pictures.planes[index].taken {
            return 0;
        }
        let style = actor_style(self.pictures.planes[index].kind);
        if style == PROP {
            return 0;
        }
        let mut pain = 0;
        if style != BARREL && self.pictures.planes[index].mode != DEATH {
            self.pictures.planes[index].see_t -= dt;
            if !self.pictures.planes[index].awake && self.pictures.planes[index].see_t <= 0.0 {
                self.pictures.planes[index].see_t = 0.28;
                if self.sees(index, eye, octree, grid, 1500.0) {
                    self.pictures.planes[index].awake = true;
                    self.pictures.planes[index].mode = CHASE;
                    self.pictures.planes[index].next_act = 0.35;
                }
            }
        }

        let mode = self.pictures.planes[index].mode;
        if mode == CHASE {
            self.step_toward(index, dt, eye, octree, grid);
            self.pictures.planes[index].next_act -= dt;
            if self.pictures.planes[index].next_act <= 0.0 {
                self.pictures.planes[index].next_act = 0.2;
                let dist = map_dist(self.pictures.planes[index].feet, eye, grid);
                let attack = match style {
                    ZOMBIE | SG => dist < 1024.0 && self.sees(index, eye, octree, grid, 1024.0),
                    IMP => dist < 1024.0 && self.sees(index, eye, octree, grid, 1024.0),
                    DEMON => dist < 80.0,
                    _ => false,
                };
                if attack {
                    self.pictures.planes[index].mode = ATTACK;
                    self.pictures.planes[index].mode_t = 0.0;
                    self.pictures.planes[index].acted = false;
                    self.pictures.planes[index].swing =
                        if style == IMP && dist > 72.0 { 1 } else { 0 };
                }
            }
        }

        if self.pictures.planes[index].mode == ATTACK && !self.pictures.planes[index].acted {
            self.pictures.planes[index].acted = true;
            pain += self.actor_attack(index, eye, octree, grid);
        }

        self.pictures.planes[index].mode_t += dt;
        let mode_t = self.pictures.planes[index].mode_t;
        let sprite = self.pictures.planes[index].sprite;
        let swing = self.pictures.planes[index].swing;
        match self.pictures.planes[index].mode {
            IDLE => {
                let n = (mode_t / 0.17) as usize;
                self.pictures.planes[index].frame = if n % 2 == 0 { b'A' } else { b'B' };
            }
            CHASE => {
                let letters = b"ABCD";
                let n = ((mode_t / 0.11) as usize) % 4;
                self.pictures.planes[index].frame = letters[n];
            }
            ATTACK => {
                self.pictures.planes[index].frame = attack_letter(sprite, swing, mode_t);
                if mode_t > attack_len(sprite, swing) {
                    self.pictures.planes[index].mode = CHASE;
                    self.pictures.planes[index].mode_t = 0.0;
                    self.pictures.planes[index].next_act = 0.45;
                    self.pictures.planes[index].acted = false;
                }
            }
            PAIN => {
                self.pictures.planes[index].frame = if sprite == "TROO" { b'J' } else { b'G' };
                if mode_t > 0.2 {
                    self.pictures.planes[index].mode = CHASE;
                    self.pictures.planes[index].mode_t = 0.0;
                }
            }
            DEATH => {
                self.pictures.planes[index].frame = death_letter(sprite, mode_t);
            }
            BURST => {
                // Doom: 5, 5, 5, 10, 10 tics.
                let letters = b"ABCDE";
                let n = if mode_t < 0.14 {
                    0
                } else if mode_t < 0.29 {
                    1
                } else if mode_t < 0.43 {
                    2
                } else if mode_t < 0.71 {
                    3
                } else {
                    4
                };
                self.pictures.planes[index].sprite = "BEXP";
                self.pictures.planes[index].frame = letters[n];
                if mode_t > 1.0 {
                    self.pictures.planes[index].taken = true;
                }
            }
            _ => {}
        }
        pain
    }

    fn step_toward(
        &mut self,
        index: usize,
        dt: f32,
        eye: Point3<f32>,
        octree: &CpuOctree,
        grid: i32,
    ) {
        let speed = self.pictures.planes[index].speed;
        if speed <= 0.0 {
            return;
        }
        let feet = self.pictures.planes[index].feet;
        let radius = self.pictures.planes[index].radius;
        let dx = eye.x - feet[0];
        let dz = eye.z - feet[2];
        let dist = (dx * dx + dz * dz).sqrt();
        if dist < 1.0e-5 {
            return;
        }
        let stop = (radius + 20.0) * (2.0 / grid as f32);
        if dist <= stop {
            return;
        }
        let wish = Vector3::new(dx / dist, 0.0, dz / dist);
        let voxel = 2.0 / grid as f32;
        let lift = 41.0 * voxel;
        let pos = Point3::new(feet[0], feet[1] + lift, feet[2]);
        let (next, _, _) = step_on_foot(octree, grid, pos, wish, speed, 0.0, dt, false, true);
        let scale = grid as f32 * 0.5;
        let blocked = self.pictures.planes.iter().enumerate().any(|(j, other)| {
            if j == index || other.taken || other.radius <= 0.0 {
                return false;
            }
            let ox = (other.feet[0] - next.x) * scale;
            let oz = (other.feet[2] - next.z) * scale;
            let need = radius + other.radius;
            ox * ox + oz * oz < need * need
        });
        if !blocked && !self.door_blocks(next, grid) {
            self.pictures.planes[index].feet = [next.x, next.y - lift, next.z];
        }
    }

    fn actor_attack(
        &mut self,
        index: usize,
        eye: Point3<f32>,
        octree: &CpuOctree,
        grid: i32,
    ) -> i32 {
        let style = actor_style(self.pictures.planes[index].kind);
        let swing = self.pictures.planes[index].swing;
        let feet = self.pictures.planes[index].feet;
        let dist = map_dist(feet, eye, grid);
        match style {
            ZOMBIE => {
                if self.sees(index, eye, octree, grid, 1024.0) {
                    (self.roll(5) + 1) as i32 * 3
                } else {
                    0
                }
            }
            SG => {
                if !self.sees(index, eye, octree, grid, 1024.0) {
                    return 0;
                }
                let mut dmg = 0;
                for _ in 0..3 {
                    dmg += (self.roll(5) + 1) as i32 * 3;
                }
                dmg
            }
            DEMON => {
                if dist < 80.0 {
                    (self.roll(8) + 1) as i32 * 3
                } else {
                    0
                }
            }
            IMP if swing == 0 && dist < 80.0 => (self.roll(8) + 1) as i32 * 3,
            IMP => {
                let voxel = 2.0 / grid as f32;
                let from = [feet[0], feet[1] + 32.0 * voxel, feet[2]];
                let dir = Vector3::new(eye.x - from[0], eye.y - from[1], eye.z - from[2]);
                if dir.magnitude2() > 1.0e-8 {
                    self.spawn_flyer(
                        Point3::new(from[0], from[1], from[2]),
                        dir.normalize(),
                        grid,
                        index,
                        IMP_FLY,
                        350.0,
                    );
                }
                0
            }
            _ => 0,
        }
    }

    fn sees(
        &self,
        index: usize,
        eye: Point3<f32>,
        octree: &CpuOctree,
        grid: i32,
        reach: f32,
    ) -> bool {
        let feet = self.pictures.planes[index].feet;
        let dist = map_dist(feet, eye, grid);
        if dist > reach || dist < 1.0 {
            return dist < 1.0;
        }
        let voxel = 2.0 / grid as f32;
        let from = Point3::new(feet[0], feet[1] + 36.0 * voxel, feet[2]);
        let dir = Vector3::new(eye.x - from.x, eye.y - from.y, eye.z - from.z);
        if dir.magnitude2() < 1.0e-8 {
            return true;
        }
        let dir = dir.normalize();
        let wall = wall_distance(octree, grid, from, dir);
        wall + 8.0 * voxel > (eye - from).magnitude()
    }

    fn spawn_flyer(
        &mut self,
        eye: Point3<f32>,
        dir: Vector3<f32>,
        grid: i32,
        owner: usize,
        kind: u8,
        speed: f32,
    ) {
        if self.flyers.len() > 48 {
            return;
        }
        let voxel = 2.0 / grid as f32;
        let ahead = if kind == ROCKET_FLY { 32.0 } else { 24.0 };
        let pos = [
            eye.x + dir.x * ahead * voxel,
            eye.y + dir.y * ahead * voxel,
            eye.z + dir.z * ahead * voxel,
        ];
        let (sprite, frame) = if kind == ROCKET_FLY {
            ("MISL", b'A')
        } else {
            ("BAL1", b'A')
        };
        self.flyers.push(Flyer {
            pos,
            dir: [dir.x, dir.y, dir.z],
            speed,
            sprite,
            frame,
            time: 0.0,
            kind,
            owner,
        });
    }

    fn puff(&mut self, pos: [f32; 3], sprite: &'static str) {
        if self.flyers.len() > 48 {
            return;
        }
        self.flyers.push(Flyer {
            pos,
            dir: [0.0, 0.0, 0.0],
            speed: 0.0,
            sprite,
            frame: b'A',
            time: 0.0,
            kind: PUFF_FLY,
            owner: usize::MAX,
        });
    }

    fn tick_flyers(&mut self, dt: f32, eye: Point3<f32>, octree: &CpuOctree, grid: i32) -> i32 {
        let bodies: Vec<(usize, [f32; 3], f32, f32)> = self
            .pictures
            .planes
            .iter()
            .enumerate()
            .filter_map(|(index, plane)| {
                if shootable(plane) {
                    Some((index, plane.feet, plane.radius, plane.tall))
                } else {
                    None
                }
            })
            .collect();
        let mut impacts = Vec::new();
        for flyer in &mut self.flyers {
            flyer.time += dt;
            if flyer.kind == PUFF_FLY {
                let letters: &[u8] = if flyer.sprite == "BAL1" {
                    b"CDE"
                } else if flyer.sprite == "MISL" {
                    b"BCD"
                } else {
                    b"ABCD"
                };
                let n = ((flyer.time / 0.08) as usize).min(letters.len() - 1);
                flyer.frame = letters[n];
                if flyer.time > 0.08 * letters.len() as f32 {
                    flyer.kind = DEAD_FLY;
                }
                continue;
            }
            let kind = flyer.kind;
            let owner = flyer.owner;
            let voxel = 2.0 / grid as f32;
            let step = flyer.speed * dt * voxel;
            let dir = Vector3::new(flyer.dir[0], flyer.dir[1], flyer.dir[2]);
            let from = Point3::new(flyer.pos[0], flyer.pos[1], flyer.pos[2]);
            let wall = wall_distance(octree, grid, from, dir);
            let mut target = None;
            let mut best = step;
            if wall < best {
                best = wall;
                target = Some(usize::MAX);
            }
            for (index, feet, radius, tall) in &bodies {
                if *index == owner {
                    continue;
                }
                if let Some(t) = hit_cylinder(from, dir, *feet, radius * voxel, tall * voxel) {
                    if t < best {
                        best = t;
                        target = Some(*index);
                    }
                }
            }
            let player_r = 16.0 * voxel;
            if owner != usize::MAX || flyer.time > 0.12 {
                let body = [eye.x, eye.y - 41.0 * voxel, eye.z];
                if let Some(t) = hit_cylinder(from, dir, body, player_r, 56.0 * voxel) {
                    if t < best {
                        best = t;
                        target = Some(usize::MAX - 1);
                    }
                }
            }
            if let Some(hit) = target {
                let pos = [
                    from.x + dir.x * best.max(0.0),
                    from.y + dir.y * best.max(0.0),
                    from.z + dir.z * best.max(0.0),
                ];
                flyer.kind = DEAD_FLY;
                impacts.push((kind, pos, hit));
                continue;
            }
            flyer.pos = [
                from.x + dir.x * step,
                from.y + dir.y * step,
                from.z + dir.z * step,
            ];
            flyer.frame = if (flyer.time / 0.1) as usize % 2 == 0 {
                b'A'
            } else {
                b'B'
            };
        }
        self.flyers.retain(|flyer| flyer.kind != DEAD_FLY);
        let mut pain = 0;
        for (kind, pos, target) in impacts {
            if kind == ROCKET_FLY {
                pain += self.blast_at(pos, None, eye, octree, grid);
                self.puff(pos, "MISL");
                continue;
            }
            if target == usize::MAX - 1 {
                pain += (self.roll(8) + 1) as i32 * 3;
            } else if target < self.pictures.planes.len() {
                let dmg = ((self.roll(8) + 1) * 3) as i32;
                pain += self.apply_damage(target, dmg, eye, octree, grid);
            }
            self.puff(pos, "BAL1");
        }
        pain
    }

    fn roll(&mut self, n: u32) -> u32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        if n == 0 {
            0
        } else {
            self.rng % n
        }
    }
}

const PUFF_FLY: u8 = 0;
const IMP_FLY: u8 = 1;
const ROCKET_FLY: u8 = 2;
const DEAD_FLY: u8 = 3;

struct Flyer {
    pos: [f32; 3],
    dir: [f32; 3],
    speed: f32,
    sprite: &'static str,
    frame: u8,
    time: f32,
    kind: u8,
    owner: usize,
}

struct GunSeq {
    frames: &'static [u8],
    hold: &'static [f32],
    flash_hold: f32,
}

fn gun_sequence(label: &str) -> GunSeq {
    match label {
        "SHOTGUN" => GunSeq {
            frames: &[0, 0, 1, 2, 3, 2, 1, 0],
            hold: &[0.09, 0.20, 0.14, 0.14, 0.11, 0.14, 0.14, 0.20],
            flash_hold: 0.14,
        },
        "CHAINGUN" => GunSeq {
            frames: &[0],
            hold: &[0.11],
            flash_hold: 0.11,
        },
        "ROCKET" => GunSeq {
            frames: &[0, 1],
            hold: &[0.23, 0.34],
            flash_hold: 0.43,
        },
        _ => GunSeq {
            frames: &[0, 1, 2, 1],
            hold: &[0.11, 0.17, 0.11, 0.14],
            flash_hold: 0.20,
        },
    }
}

fn kind_stats(kind: i16) -> (i16, f32, f32, f32, u8) {
    match kind {
        2035 => (20, 10.0, 42.0, 0.0, BARREL),
        // Chase states last 4 tics, so speed 8 is 70 map units a second, not 280.
        3004 | 18 => (20, 20.0, 56.0, 70.0, ZOMBIE),
        9 | 19 => (30, 20.0, 56.0, 70.0, SG),
        3001 | 20 => (60, 20.0, 56.0, 70.0, IMP),
        3002 | 58 | 21 => (150, 30.0, 56.0, 88.0, DEMON),
        3003 => (1000, 24.0, 64.0, 70.0, IMP),
        3005 => (400, 31.0, 56.0, 70.0, IMP),
        3006 => (100, 16.0, 56.0, 70.0, ZOMBIE),
        16 => (4000, 40.0, 110.0, 70.0, IMP),
        _ => (0, 0.0, 0.0, 0.0, PROP),
    }
}

fn actor_style(kind: i16) -> u8 {
    kind_stats(kind).4
}

fn shootable(plane: &Plane) -> bool {
    !plane.taken && plane.health > 0 && plane.mode != DEATH && plane.mode != BURST
}

fn map_dist(feet: [f32; 3], eye: Point3<f32>, grid: i32) -> f32 {
    let scale = grid as f32 * 0.5;
    let dx = (eye.x - feet[0]) * scale;
    let dz = (eye.z - feet[2]) * scale;
    (dx * dx + dz * dz).sqrt()
}

fn attack_letter(sprite: &str, swing: u8, t: f32) -> u8 {
    if sprite == "TROO" && swing == 1 {
        return if t < 0.28 { b'H' } else { b'I' };
    }
    if sprite == "TROO" {
        return if t < 0.2 {
            b'E'
        } else if t < 0.4 {
            b'F'
        } else {
            b'G'
        };
    }
    if t < 0.25 {
        b'E'
    } else {
        b'F'
    }
}

fn attack_len(sprite: &str, swing: u8) -> f32 {
    if sprite == "TROO" && swing == 0 {
        0.6
    } else {
        0.5
    }
}

fn death_letter(sprite: &str, t: f32) -> u8 {
    let frames: &[u8] = if sprite == "TROO" { b"KLMNO" } else { b"HIJKL" };
    frames[((t / 0.12) as usize).min(frames.len() - 1)]
}

fn hit_cylinder(
    eye: Point3<f32>,
    dir: Vector3<f32>,
    origin: [f32; 3],
    radius: f32,
    height: f32,
) -> Option<f32> {
    if radius <= 0.0 {
        return None;
    }
    let ox = eye.x - origin[0];
    let oz = eye.z - origin[2];
    let a = dir.x * dir.x + dir.z * dir.z;
    if a < 1.0e-8 {
        return None;
    }
    let b = 2.0 * (ox * dir.x + oz * dir.z);
    let c = ox * ox + oz * oz - radius * radius;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let t0 = (-b - s) / (2.0 * a);
    let t1 = (-b + s) / (2.0 * a);
    if c < 0.0 && eye.y >= origin[1] && eye.y <= origin[1] + height {
        return Some(0.001);
    }
    let inside = |t: f32| {
        if t < 0.0004 {
            return false;
        }
        let y = eye.y + dir.y * t;
        y >= origin[1] && y <= origin[1] + height
    };
    if inside(t0) {
        Some(t0)
    } else if inside(t1) {
        Some(t1)
    } else {
        None
    }
}

fn push_view_sprite(
    out: &mut Vec<SpriteVert>,
    clip: &Clip,
    screen_w: f32,
    screen_h: f32,
    bob_x: f32,
    bob_y: f32,
) {
    // Doom pins the weapon origin at sy = 32 on a 200-tall view, then the patch
    // offset places each frame. The muzzle flash uses that same origin, so it
    // sits on the barrel instead of on the grip.
    let scale = screen_h / 200.0;
    let x_from_center = 1.0 - 160.0 - clip.left as f32;
    let x0 = x_from_center * scale / screen_w * 2.0 + bob_x;
    let x1 = x0 + clip.px_w * scale / screen_w * 2.0;
    let top_px = 32.0 - clip.top as f32;
    let lift = 64.0 / screen_h * 2.0;
    let y1 = 1.0 - top_px / 100.0 + lift + bob_y;
    let y0 = y1 - clip.px_h / 100.0;
    let ndc = [[x0, y0], [x1, y0], [x0, y1], [x1, y1]];
    push_quad(out, ndc, [0.0; 4], clip.uv);
}

fn spread(dir: Vector3<f32>, pellet: usize) -> Vector3<f32> {
    let yaw = (pellet as f32 - 3.0) * 0.03;
    let pitch = ((pellet % 3) as f32 - 1.0) * 0.015;
    let (s, c) = yaw.sin_cos();
    Vector3::new(dir.x * c + dir.z * s, dir.y + pitch, -dir.x * s + dir.z * c).normalize()
}

fn is_item(kind: i16) -> bool {
    matches!(
        kind,
        5 | 6
            | 8
            | 13
            | 38
            | 39
            | 40
            | 2001
            | 2002
            | 2003
            | 2007
            | 2008
            | 2011
            | 2012
            | 2013
            | 2014
            | 2015
            | 2018
            | 2019
            | 2046
            | 2048
            | 2049
    )
}

fn project_ndc(vp: Vector4<f32>, fov: f32, aspect_h_over_w: f32) -> [f32; 2] {
    let s = 1.0 / ((fov * 0.5) * std::f32::consts::PI / 180.0).tan();
    let z = -vp.z;
    [aspect_h_over_w * s * vp.x / z, s * vp.y / z]
}

fn push_door(
    out: &mut Vec<SpriteVert>,
    eye: Point3<f32>,
    view: &Matrix4<f32>,
    fov: f32,
    aspect: f32,
    door: &Door,
) {
    let rise = door.lift * door.height;
    let y0 = door.a[1] + rise;
    let y1 = y0 + door.height;
    let corners = [
        Point3::new(door.a[0], y0, door.a[2]),
        Point3::new(door.b[0], y0, door.b[2]),
        Point3::new(door.a[0], y1, door.a[2]),
        Point3::new(door.b[0], y1, door.b[2]),
    ];
    let mid = Point3::new(
        (door.a[0] + door.b[0]) * 0.5,
        (y0 + y1) * 0.5,
        (door.a[2] + door.b[2]) * 0.5,
    );
    let origin_vp = view * mid.to_homogeneous();
    if -origin_vp.z < 0.02 {
        return;
    }
    let mut ndc = [[0.0; 2]; 4];
    let mut dist = [0.0; 4];
    for (i, corner) in corners.iter().enumerate() {
        let mut vp = view * corner.to_homogeneous();
        if -vp.z < 0.05 {
            vp.z = origin_vp.z;
        }
        ndc[i] = project_ndc(vp, fov, aspect);
        dist[i] = (*corner - eye).magnitude().max(0.05);
    }
    push_quad(out, ndc, dist, door.uv);
}

fn closest_xz(px: f32, pz: f32, door: &Door) -> (f32, f32, f32) {
    let abx = door.b[0] - door.a[0];
    let abz = door.b[2] - door.a[2];
    let len2 = abx * abx + abz * abz;
    let t = if len2 < 1.0e-12 {
        0.0
    } else {
        (((px - door.a[0]) * abx + (pz - door.a[2]) * abz) / len2).clamp(0.0, 1.0)
    };
    let cx = door.a[0] + abx * t;
    let cz = door.a[2] + abz * t;
    let dx = cx - px;
    let dz = cz - pz;
    ((dx * dx + dz * dz).sqrt(), cx, cz)
}

fn segment_dist_xz(px: f32, pz: f32, door: &Door) -> f32 {
    closest_xz(px, pz, door).0
}

fn push_quad(out: &mut Vec<SpriteVert>, ndc: [[f32; 2]; 4], dist: [f32; 4], uv: [f32; 4]) {
    // bl, br, tl, tr
    let uvs = [
        [uv[0], uv[3]],
        [uv[2], uv[3]],
        [uv[0], uv[1]],
        [uv[2], uv[1]],
    ];
    let order = [0, 1, 2, 2, 1, 3];
    for i in order {
        out.push(SpriteVert {
            pos: ndc[i],
            uv: uvs[i],
            dist: dist[i],
            _pad: 0.0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stimpack_is_a_pickup_and_a_barrel_is_not() {
        assert!(is_item(2011));
        assert!(is_item(2001));
        assert!(!is_item(2035));
        assert!(!is_item(3004));
    }

    fn bare(planes: Vec<Plane>) -> Pictures {
        Pictures {
            atlas: vec![0, 0, 0, 0],
            atlas_size: 1,
            planes,
            guns: Vec::new(),
            clips: Vec::new(),
            names: std::collections::HashMap::new(),
            doors: Vec::new(),
            voxel: 2.0 / 64.0,
        }
    }

    fn actor(kind: i16, feet: [f32; 3]) -> Plane {
        let (health, radius, tall, speed, _) = super::kind_stats(kind);
        Plane {
            feet,
            kind,
            taken: false,
            sprite: if kind == 2035 { "BAR1" } else { "POSS" },
            frame: b'A',
            health,
            radius,
            tall,
            speed,
            mode: super::IDLE,
            mode_t: 0.0,
            next_act: 0.4,
            see_t: 10.0,
            awake: false,
            acted: false,
            swing: 0,
        }
    }

    #[test]
    fn left_click_hurts_the_monster_and_plays_the_gun() {
        let mut play = Play::new(bare(vec![actor(3004, [0.0, 0.0, -0.4])]));
        let tree = CpuOctree::new(0);
        let eye = Point3::new(0.0, 0.2, 0.0);
        let look = Vector3::new(0.0, 0.0, -1.0);
        play.fire(1.0, eye, look, &tree, 256);
        assert!(play.pictures.planes[0].health < 20);
        assert!(!play.pictures.planes[0].taken);
        assert!(play.gun_busy);
        assert_eq!(play.ammo, 49);
        play.fire(1.05, eye, look, &tree, 256);
        assert_eq!(play.ammo, 49, "the pistol waits out its animation");
        play.tick(0.2, eye, &tree, 256);
        assert!(play.gun_step >= 1, "the pistol advances off the idle frame");
    }

    #[test]
    fn a_shot_barrel_explodes_and_sets_off_its_neighbour() {
        let mut play = Play::new(bare(vec![
            actor(2035, [0.0, 0.0, -0.8]),
            actor(2035, [0.15, 0.0, -0.8]),
        ]));
        let tree = CpuOctree::new(0);
        let eye = Point3::new(0.0, 0.4, 0.0);
        let pain = play.strike(0, 50, eye, &tree, 64);
        assert_eq!(play.pictures.planes[0].mode, super::BURST);
        assert_eq!(play.pictures.planes[0].sprite, "BEXP");
        assert_eq!(play.pictures.planes[1].mode, super::BURST);
        assert!(pain > 40, "the blast reaches the player, pain={pain}");
        play.tick(1.1, eye, &tree, 64);
        assert!(
            play.pictures.planes[0].taken,
            "the explosion finishes and clears"
        );
    }

    #[test]
    fn an_awake_zombie_walks_toward_the_player() {
        let mut zombie = actor(3004, [0.0, 0.0, 0.0]);
        zombie.awake = true;
        zombie.mode = super::CHASE;
        zombie.next_act = 5.0;
        let mut play = Play::new(bare(vec![zombie]));
        let tree = CpuOctree::new(0);
        let eye = Point3::new(0.0, 0.2, -2.0);
        play.tick(0.2, eye, &tree, 256);
        assert!(
            play.pictures.planes[0].feet[2] < -0.05,
            "moved toward the player, z={}",
            play.pictures.planes[0].feet[2]
        );
        assert_ne!(play.pictures.planes[0].frame, b'A');
    }

    #[test]
    fn a_closed_door_plane_blocks_then_rises() {
        let voxel = 2.0 / 256.0;
        let mut pictures = bare(Vec::new());
        pictures.voxel = voxel;
        pictures.doors.push(Door {
            sector: 1,
            a: [-0.2, 0.0, 0.0],
            b: [0.2, 0.0, 0.0],
            height: 72.0 * voxel,
            lift: 0.0,
            motion: DoorMotion::Shut,
            phase_t: 0.0,
            uv: [0.0, 0.0, 1.0, 1.0],
        });
        let mut play = Play::new(pictures);
        let tree = CpuOctree::new(0);
        let eye = Point3::new(0.0, 41.0 * voxel, -0.05);
        assert!(play.door_blocks(eye, 256));
        play.use_facing(eye, Vector3::new(0.0, 0.0, -1.0), 256);
        assert_eq!(play.pictures.doors[0].motion, DoorMotion::Shut);
        play.use_facing(eye, Vector3::new(0.0, 0.0, 1.0), 256);
        assert_eq!(play.pictures.doors[0].motion, DoorMotion::Opening);
        play.tick(1.0, eye, &tree, 256);
        assert!(play.pictures.doors[0].lift > 0.99);
        assert!(!play.door_blocks(eye, 256));
    }
}
