use cgmath::*;
use std::time::Instant;
use wgpu::util::DeviceExt;
use winit::{
    event::*,
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

mod adaptive;
mod room_play;
mod app;
mod block_world;
mod compute;
mod cpu_octree;
mod doom;
mod gpu;
mod octree;
mod oidn_denoise;
mod play;
mod procedural;
mod render;
mod wad;
mod world;
use adaptive::*;
use app::*;
use block_world::*;
use compute::*;
use cpu_octree::*;
use doom::*;
use gpu::*;
use octree::*;
use play::*;
use procedural::*;
use render::*;
use world::*;

fn wants_gzdoom() -> bool {
    std::env::args().any(|arg| arg == "--gzdoom" || arg == "gzdoom")
}

fn gzdoom_binary() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("GZDOOM") {
        let path = std::path::PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let bundles = [
        "/Applications/GZDoom.app/Contents/MacOS/gzdoom",
        "/Applications/GZDoom.app/Contents/MacOS/GZDoom",
    ];
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("../gzdoom/build/gzdoom.app/Contents/MacOS/gzdoom"));
    }
    candidates.extend(bundles.iter().map(std::path::PathBuf::from));
    if let Some(home) = home {
        for bundle in bundles {
            candidates.push(home.join(bundle.trim_start_matches('/')));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// Real GZDoom, pointed at the IWAD directory this project already reads.
fn run_gzdoom() -> i32 {
    let Some(bin) = gzdoom_binary() else {
        eprintln!("GZDoom is not installed.");
        eprintln!("Install it with: brew install --cask gzdoom");
        return 1;
    };
    let mut cmd = std::process::Command::new(&bin);
    if let Some(wad) = wad::find_wad() {
        if let Some(dir) = wad.parent() {
            println!("GZDoom IWAD directory {}", dir.display());
            cmd.env("DOOMWADDIR", dir);
        }
        cmd.arg("-iwad").arg(&wad);
    }
    println!("Running {}", bin.display());
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("Could not start GZDoom: {err}");
            1
        }
    }
}

#[tokio::main]
async fn main() {
    println!("octree-tracer v0.1.0");
    if wants_gzdoom() {
        std::process::exit(run_gzdoom());
    }

    env_logger::init();
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().build(&event_loop).unwrap();

    let mut app = pollster::block_on(App::new(&window));

    let now = Instant::now();
    event_loop.run(move |event, _, control_flow| {
        app.render.egui_platform.handle_event(&event);
        app.input(&window, &event);
        match event {
            Event::RedrawRequested(_) => {
                match app.render.render(&app.gpu, &window) {
                    Ok(_) => {}
                    // Reconfigure the surface if lost
                    Err(wgpu::SurfaceError::Lost) => app.render.resize(&app.gpu, app.render.size),
                    // The system is out of memory, we should probably quit
                    Err(wgpu::SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,
                    // All other errors (Outdated, Timeout) should be resolved by the next frame
                    Err(e) => eprintln!("{:?}", e),
                }
                app.update(now.elapsed().as_secs_f64());
            }
            Event::MainEventsCleared => {
                // RedrawRequested will only trigger once, unless we manually
                // request it.
                window.request_redraw();
            }
            Event::WindowEvent {
                ref event,
                window_id,
            } if window_id == window.id() => {
                match event {
                    WindowEvent::Resized(physical_size) => {
                        app.render.resize(&app.gpu, *physical_size);
                    }
                    WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                        // new_inner_size is &&mut so we have to dereference it twice
                        app.render.resize(&app.gpu, **new_inner_size);
                    }
                    WindowEvent::CloseRequested
                    | WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                state: ElementState::Pressed,
                                virtual_keycode: Some(VirtualKeyCode::Q),
                                ..
                            },
                        ..
                    } => *control_flow = ControlFlow::Exit,
                    _ => {}
                }
            }
            _ => {}
        }
    });
}

pub struct Input {
    pub(crate) forward: bool,
    pub(crate) backward: bool,
    pub(crate) right: bool,
    pub(crate) left: bool,
    pub(crate) up: bool,
    pub(crate) down: bool,
    pub(crate) boost: bool,
    pub(crate) fire: bool,
    pub(crate) use_door: bool,
    pub(crate) mouse_delta: Vector2<f32>,
}

impl Input {
    fn new() -> Self {
        Self {
            forward: false,
            backward: false,
            right: false,
            left: false,
            up: false,
            down: false,
            boost: false,
            fire: false,
            use_door: false,
            mouse_delta: Vector2::zero(),
        }
    }
}

pub struct Settings {
    octree_depth: u32,
    fov: f32,
    sensitivity: f32,
}

pub struct Character {
    pos: Point3<f32>,
    look: Vector3<f32>,
    cursour_grabbed: bool,
    speed: f32,
}

impl Character {
    fn new() -> Self {
        Self {
            pos: Point3::new(0.1, 0.2, -1.5),
            look: -Vector3::new(0.0, 0.0, -1.5),
            cursour_grabbed: true,
            speed: -5.0,
        }
    }
}

fn create_proj_matrix(fov: f32, aspect: f32) -> Matrix4<f32> {
    let s = 1.0 / ((fov / 2.0) * (std::f32::consts::PI / 180.0)).tan();
    Matrix4::new(
        aspect * s,
        0.0,
        0.0,
        0.0,
        //
        0.0,
        s,
        0.0,
        0.0,
        //
        0.0,
        0.0,
        -1.0,
        0.0,
        //
        0.0,
        0.0,
        0.0,
        1.0,
    )
}
