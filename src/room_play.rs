//! Doom's own actors, pickups, projectiles, and moving sectors.
//!
//! The octree keeps the walls. `room` runs the engine and hands back the
//! voxel models, leftover billboards, door slabs, and the view weapon.

use super::*;
use std::ffi::{c_char, CString};

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ThingVert {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub atlas: [f32; 4],
    pub tint: [f32; 3],
    pub light: f32,
}

pub struct ThingMesh {
    pub verts: Vec<ThingVert>,
    pub world_indices: Vec<u32>,
    pub gun_indices: Vec<u32>,
    pub atlas: Option<(u32, Vec<u8>)>,
}

pub struct RoomPlay {
    _args: Vec<CString>,
    _argv: Vec<*mut c_char>,
    held: [bool; 7],
}

unsafe impl Send for RoomPlay {}

impl RoomPlay {
    pub fn start(wad: &std::path::Path) -> Self {
        room::hw::use_external_walls(true);
        room::platform::init_start_time();
        let wad = wad.to_string_lossy().into_owned();
        let args = vec![
            CString::new("octree-tracer").unwrap(),
            CString::new("-iwad").unwrap(),
            CString::new(wad).unwrap(),
            CString::new("-skill").unwrap(),
            CString::new("3").unwrap(),
            CString::new("-warp").unwrap(),
            CString::new("1").unwrap(),
            CString::new("1").unwrap(),
        ];
        let mut argv: Vec<*mut c_char> = args.iter().map(|s| s.as_ptr() as *mut c_char).collect();
        argv.push(std::ptr::null_mut());
        unsafe {
            room::doom::doomgeneric::doomgeneric_Create((args.len()) as _, argv.as_mut_ptr());
        }
        room::platform::set_pointer_locked(true);
        println!("Doom actors are running in room");
        Self {
            _args: args,
            _argv: argv,
            held: [false; 7],
        }
    }

    pub fn sync(&mut self, input: &Input, grabbed: bool) {
        room::platform::set_pointer_locked(grabbed);
        let run = input.boost || input.down;
        let use_down = input.use_door || input.up;
        let now = [
            input.forward,
            input.backward,
            input.left,
            input.right,
            run,
            use_down,
            input.fire,
        ];
        let keys = [
            b'w',
            b's',
            b'a',
            b'd',
            room::doom::doomkeys::KEY_RSHIFT,
            room::doom::doomkeys::KEY_USE,
            room::doom::doomkeys::KEY_FIRE,
        ];
        for i in 0..now.len() {
            if now[i] != self.held[i] {
                room::platform::push_doom_key(now[i], keys[i]);
                self.held[i] = now[i];
            }
        }
        if grabbed && (input.mouse_delta.x != 0.0 || input.mouse_delta.y != 0.0) {
            room::platform::on_mouse_motion(input.mouse_delta.x as f64, input.mouse_delta.y as f64);
        }
        let buttons = if input.fire { 1 } else { 0 };
        room::platform::set_mouse_buttons(buttons);
    }

    pub fn tick(&mut self) {
        unsafe {
            room::doom::d_main::doomgeneric_Tick();
        }
    }

    pub fn view(&self, frame: &crate::wad::MapFrame) -> Option<(Point3<f32>, Vector3<f32>)> {
        if !room::hw::active() {
            return None;
        }
        unsafe {
            let x = room::doom::r_main::viewx as f32 / 65536.0;
            let y = room::doom::r_main::viewy as f32 / 65536.0;
            let z = room::doom::r_main::viewz as f32 / 65536.0;
            let ang = room::doom::r_main::viewangle as f32 * std::f32::consts::TAU / 4294967296.0;
            let pitch = room::platform::look_pitch();
            let (sp, cp) = pitch.sin_cos();
            let look = Vector3::new(ang.cos() * cp, sp, -ang.sin() * cp);
            let pos = place(frame, [x, z, -y]);
            Some((Point3::new(pos[0], pos[1], pos[2]), look))
        }
    }

    pub fn mesh(&self, frame: &crate::wad::MapFrame) -> Option<ThingMesh> {
        if !room::hw::active() {
            return None;
        }
        let scene = unsafe { room::hw::build_scene() };
        let atlas = scene.atlas.clone();
        let mut verts = Vec::with_capacity(scene.verts.len());
        for vert in &scene.verts {
            let pos = place(frame, vert.pos);
            verts.push(ThingVert {
                pos,
                uv: vert.uv,
                atlas: vert.atlas,
                tint: vert.tint,
                light: vert.light,
            });
        }
        let sky = scene.sky_indices as usize;
        let world = scene.world_indices as usize;
        let world_indices = scene.indices.get(sky..world).unwrap_or(&[]).to_vec();
        let gun_indices = scene.indices.get(world..).unwrap_or(&[]).to_vec();
        room::hw::recycle(scene);
        static LOGGED: std::sync::Once = std::sync::Once::new();
        let world_tris = world_indices.len() / 3;
        let gun_tris = gun_indices.len() / 3;
        LOGGED.call_once(|| {
            println!("room scene: {world_tris} world triangles, {gun_tris} gun triangles");
        });
        Some(ThingMesh {
            verts,
            world_indices,
            gun_indices,
            atlas,
        })
    }
}

/// Room's world is Doom X, height, and −Y. The octree cube starts at the map's
/// minimum corner and uses the same cell size as a map unit.
fn place(frame: &crate::wad::MapFrame, doom: [f32; 3]) -> [f32; 3] {
    let n = frame.grid.max(1) as f32;
    let cell = |v: f32| -1.0 + (v + 0.5) * 2.0 / n;
    [
        cell(doom[0] - frame.min_x as f32),
        cell(doom[1] - frame.min_h as f32),
        cell(frame.max_y as f32 + doom[2]),
    ]
}
