//! # stickfell_app
//!
//! Opens a window, ticks a [`Game`] at a fixed rate, and renders it with
//! [`stickfell_render`]. Also provides [`capture_headless`] for rendering a game
//! to a PNG without a window (screenshots, visual tests, CI).

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec2;
use stickfell_render::{Frame, Renderer, wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, CustomCursor, Window, WindowId};

pub use winit::keyboard::KeyCode;
pub use winit::window::CursorIcon;

/// Mouse buttons tracked by [`Input`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mouse {
    Left = 0,
    Right = 1,
    Middle = 2,
}

/// Keyboard and mouse state as seen by one fixed tick.
#[derive(Default, Clone)]
pub struct Input {
    held: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    mouse_held: [bool; 3],
    mouse_pressed: [bool; 3],
    /// Cursor position in physical pixels (origin top-left).
    pub mouse: Vec2,
    /// Output size in physical pixels.
    pub screen: Vec2,
    /// Text typed since the previous tick.
    pub text: String,
}

impl Input {
    pub fn down(&self, k: KeyCode) -> bool {
        self.held.contains(&k)
    }
    /// Pressed since the previous tick.
    pub fn pressed(&self, k: KeyCode) -> bool {
        self.pressed.contains(&k)
    }
    pub fn any_pressed(&self) -> bool {
        !self.pressed.is_empty()
    }
    pub fn mouse_down(&self, b: Mouse) -> bool {
        self.mouse_held[b as usize]
    }
    pub fn mouse_pressed(&self, b: Mouse) -> bool {
        self.mouse_pressed[b as usize]
    }
    /// Forgets the mouse buttons being held (the system kept their release,
    /// e.g. after dragging the window).
    fn release_mouse(&mut self) {
        self.mouse_held = [false; 3];
    }
    fn clear_edges(&mut self) {
        self.pressed.clear();
        self.mouse_pressed = [false; 3];
        self.text.clear();
    }
    fn press(&mut self, k: KeyCode) {
        self.held.insert(k);
        self.pressed.insert(k);
    }
    fn release(&mut self, k: KeyCode) {
        self.held.remove(&k);
    }
}

pub trait Game: 'static {
    /// Fixed-rate update.
    fn tick(&mut self, input: &Input);
    /// Fill the draw list. `screen` is the output size in pixels. `alpha`
    /// (0..1) is how far real time has moved past the last tick toward the
    /// next one: blend the previous and current tick by it for smooth motion
    /// on displays faster than the tick rate. `dt` is real seconds since the
    /// last draw.
    fn draw(&mut self, frame: &mut Frame, screen: Vec2, alpha: f32, dt: f32);
    fn wants_exit(&self) -> bool {
        false
    }
    /// Multiplier on the tick rate (1 = `AppConfig::tick_hz`). Online games
    /// use a value slightly below 1 to let a lagging peer catch up smoothly
    /// instead of skipping whole ticks.
    fn tick_rate(&self) -> f64 {
        1.0
    }
    /// Whether the OS cursor should be shown (games may draw their own).
    fn cursor_visible(&self) -> bool {
        true
    }
    /// Whether the mouse should be kept inside the window (it's let go
    /// whenever the window loses focus, and taken back when it regains it).
    fn cursor_captured(&self) -> bool {
        false
    }
    /// The mouse pointer's shape while it's shown (e.g. a hand over buttons).
    fn cursor_icon(&self) -> CursorIcon {
        CursorIcon::Default
    }
    /// Something to do to the window this frame, for a game that draws its
    /// own title bar (see `AppConfig::decorations`).
    fn window_action(&mut self) -> Option<WindowAction> {
        None
    }
}

/// What a game with its own title bar can ask of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    /// Move the window with the mouse (the left button is down on the
    /// game's title bar).
    Drag,
    Minimize,
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub title: String,
    pub size: (u32, u32),
    pub tick_hz: f64,
    /// PNG for the title bar icon (small, e.g. 32 px).
    pub window_icon: Option<&'static [u8]>,
    /// PNG for the taskbar icon on Windows (bigger, e.g. 64 px).
    pub taskbar_icon: Option<&'static [u8]>,
    /// The system's title bar and border. Without them the window is a fixed
    /// `size` in logical pixels (shrunk to fit small screens), centred, and
    /// the game draws its own title bar (see `Game::window_action`).
    pub decorations: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { title: "Stickfell".into(), size: (1280, 720), tick_hz: 60.0, window_icon: None, taskbar_icon: None, decorations: true }
    }
}

/// Decodes a PNG into a window icon (None if it isn't a valid 8-bit RGBA PNG).
fn load_icon(bytes: &[u8]) -> Option<winit::window::Icon> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    winit::window::Icon::from_rgba(buf, info.width, info.height).ok()
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    renderer: Renderer,
}

/// How far back (seconds) `Frame::mouse_trail` reaches, and at most how many
/// moves it keeps.
const TRAIL_SECS: f32 = 0.05;
const TRAIL_MAX: usize = 512;

struct Runner<G: Game> {
    cfg: AppConfig,
    game: G,
    gfx: Option<Gfx>,
    input: Input,
    last: Instant,
    acc: f64,
    start: Instant,
    frame: Frame,
    error: Option<String>,
    cursor_shown: bool,
    /// Whether the cursor is currently confined to the window, and whether
    /// the window has focus (the cursor is only confined while it does).
    cursor_grabbed: bool,
    focused: bool,
    /// An invisible cursor image, used to hide the pointer.
    blank_cursor: Option<CustomCursor>,
    /// The pointer shape last set.
    cursor_icon: CursorIcon,
    /// Recent cursor moves (when, where), for `Frame::mouse_trail`.
    trail: VecDeque<(Instant, Vec2)>,
    /// STICKFELL_PROFILE=1: (frames, acquire, render, present, game) seconds, reported every 3 s.
    profile: Option<(Instant, u32, f64, f64, f64, f64)>,
}

fn pick_srgb(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(formats[0])
}

impl<G: Game> Runner<G> {
    fn init_gfx(&mut self, el: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let mut attrs = Window::default_attributes()
            .with_title(self.cfg.title.clone())
            .with_inner_size(PhysicalSize::new(self.cfg.size.0, self.cfg.size.1))
            .with_window_icon(self.cfg.window_icon.and_then(load_icon))
            // (shown once there's a first frame in it: a window that's up
            // but not drawing yet looks frozen)
            .with_visible(false);
        if !self.cfg.decorations {
            // a fixed size in logical pixels, no bigger than most of the
            // screen (same shape), in the middle of it
            let (w, h) = (self.cfg.size.0 as f64, self.cfg.size.1 as f64);
            let mut size = PhysicalSize::new(w, h);
            let mut pos = None;
            if let Some(m) = el.primary_monitor() {
                let (scale, area) = (m.scale_factor(), m.size());
                let fit = (area.width as f64 * 0.85 / (w * scale)).min(area.height as f64 * 0.85 / (h * scale)).min(1.0);
                size = PhysicalSize::new(w * scale * fit, h * scale * fit);
                let at = m.position();
                pos = Some(winit::dpi::PhysicalPosition::new(
                    at.x as f64 + (area.width as f64 - size.width) * 0.5,
                    at.y as f64 + (area.height as f64 - size.height) * 0.5,
                ));
            }
            attrs = attrs.with_decorations(false).with_resizable(false).with_inner_size(size);
            if let Some(p) = pos {
                attrs = attrs.with_position(p);
            }
        }
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_taskbar_icon(self.cfg.taskbar_icon.and_then(load_icon))
        };
        let window = Arc::new(el.create_window(attrs)?);
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        let t_window = ms(self.start);
        // On Windows, DX12 presents through the desktop compositor's flip model
        // and paces frames evenly in a window; windowed Vulkan there can miss
        // vblanks (e.g. an uneven ~123 fps on a 144 Hz display). Try the
        // preferred backend first, then anything that works. WGPU_BACKEND
        // still overrides.
        let preferred = if cfg!(windows) { wgpu::Backends::DX12 } else { wgpu::Backends::all() };
        let mut found = None;
        for backends in [preferred, wgpu::Backends::all()] {
            let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
            desc.backends = backends;
            let instance = wgpu::Instance::new(desc.with_env());
            let surface = instance.create_surface(window.clone())?;
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            }));
            if let Ok(adapter) = adapter {
                found = Some((instance, surface, adapter));
                break;
            }
        }
        let (_instance, surface, adapter) = found.ok_or("no usable GPU adapter")?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("stickfell device"),
            ..Default::default()
        }))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface not supported by adapter")?;
        config.format = pick_srgb(&caps.formats);
        // Never vsync: frames go out as soon as they're drawn (no waiting on
        // the display), and the fixed-tick interpolation keeps motion smooth.
        let modes = &caps.present_modes;
        config.present_mode = [wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox]
            .into_iter()
            .find(|m| modes.contains(m))
            .unwrap_or(wgpu::PresentMode::AutoNoVsync);
        surface.configure(&device, &config);
        let t_gpu = ms(self.start);
        // (shaders compile on another thread: the game can draw a loading
        // screen meanwhile, see `Frame::shaders_ready`)
        let renderer = Renderer::preloading(&device, &queue, config.format);
        let info = adapter.get_info();
        eprintln!("[stickfell] startup: window {t_window:.0} ms, GPU {t_gpu:.0} ms, renderer {:.0} ms (since launch)", ms(self.start));
        eprintln!("[stickfell] {} ({:?}), surface {:?}, present {:?}", info.name, info.backend, config.format, config.present_mode);
        self.gfx = Some(Gfx { window, surface, config, device, renderer });
        self.last = Instant::now();
        Ok(())
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.25);
        self.last = now;
        self.acc += dt;
        let step = 1.0 / (self.cfg.tick_hz * self.game.tick_rate().clamp(0.5, 1.5));
        if let Some(gfx) = &self.gfx {
            self.input.screen = Vec2::new(gfx.config.width as f32, gfx.config.height as f32);
            let want = self.game.cursor_visible();
            let icon = self.game.cursor_icon();
            if want && self.cursor_shown && icon != self.cursor_icon {
                gfx.window.set_cursor(icon);
                self.cursor_icon = icon;
            }
            if want != self.cursor_shown {
                // Hidden = a fully transparent cursor image, not
                // `set_cursor_visible(false)`: on Windows winit pins a hidden,
                // confined cursor to the middle of the window, and the
                // pointer has to keep moving (the game draws its own).
                if want {
                    gfx.window.set_cursor(icon);
                    self.cursor_icon = icon;
                } else {
                    let blank = self.blank_cursor.get_or_insert_with(|| {
                        let src = CustomCursor::from_rgba(vec![0u8; 4], 1, 1, 0, 0).expect("1x1 cursor");
                        el.create_custom_cursor(src)
                    });
                    gfx.window.set_cursor(blank.clone());
                }
                self.cursor_shown = want;
            }
            let grab = self.game.cursor_captured() && self.focused;
            if grab != self.cursor_grabbed {
                let mode = if grab { CursorGrabMode::Confined } else { CursorGrabMode::None };
                // (Confined only: Locked would freeze the pointer in place)
                if let Err(e) = gfx.window.set_cursor_grab(mode) {
                    if grab {
                        eprintln!("[stickfell] couldn't capture the mouse: {e}");
                    }
                }
                self.cursor_grabbed = grab;
            }
        }
        while self.acc >= step {
            self.game.tick(&self.input);
            self.input.clear_edges();
            self.acc -= step;
        }
        if self.game.wants_exit() {
            el.exit();
            return;
        }
        if let (Some(a), Some(gfx)) = (self.game.window_action(), &self.gfx) {
            match a {
                WindowAction::Drag => {
                    let _ = gfx.window.drag_window();
                    // (the system keeps the button's release while it moves the window)
                    self.input.release_mouse();
                }
                WindowAction::Minimize => gfx.window.set_minimized(true),
            }
        }
        let Some(gfx) = self.gfx.as_mut() else { return };
        let (w, h) = (gfx.config.width, gfx.config.height);
        self.frame.clear_lists();
        self.frame.time = (now - self.start).as_secs_f32();
        self.frame.mouse = self.input.mouse;
        while self.trail.front().is_some_and(|(t, _)| now.saturating_duration_since(*t).as_secs_f32() > TRAIL_SECS) {
            self.trail.pop_front();
        }
        self.frame.mouse_trail.clear();
        self.frame.mouse_trail.extend(self.trail.iter().map(|(t, p)| (*p, now.saturating_duration_since(*t).as_secs_f32())));
        if let Some((last, _)) = self.frame.mouse_trail.last() {
            // (always ending exactly where the cursor is now)
            if *last != self.input.mouse {
                self.frame.mouse_trail.push((self.input.mouse, 0.0));
            }
        }
        let alpha = (self.acc / step).clamp(0.0, 1.0) as f32;
        let was_ready = self.frame.shaders_ready;
        self.frame.shaders_ready = gfx.renderer.ready();
        if self.frame.shaders_ready && !was_ready {
            eprintln!("[stickfell] startup: shaders ready {:.0} ms after launch", self.start.elapsed().as_secs_f64() * 1e3);
        }
        self.game.draw(&mut self.frame, Vec2::new(w as f32, h as f32), alpha, dt as f32);

        let t_game = now.elapsed().as_secs_f64();
        let t0 = Instant::now();
        let tex = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.device, &gfx.config);
                return;
            }
            _ => return,
        };
        let t_acquire = t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        let view = tex.texture.create_view(&Default::default());
        gfx.renderer.render(&self.frame, &view, w, h);
        let t_render = t1.elapsed().as_secs_f64();
        let t2 = Instant::now();
        gfx.renderer.queue().present(tex);
        let t_present = t2.elapsed().as_secs_f64();
        if let Some(p) = &mut self.profile {
            p.1 += 1;
            p.2 += t_acquire;
            p.3 += t_render;
            p.4 += t_present;
            p.5 += t_game;
            let secs = p.0.elapsed().as_secs_f64();
            if secs >= 3.0 {
                let n = p.1.max(1) as f64 * 0.001;
                eprintln!(
                    "[stickfell] {:.0} fps {}x{} {:?} | per frame: game {:.2} ms, acquire {:.2} ms, render {:.2} ms, present {:.2} ms",
                    p.1 as f64 / secs,
                    w,
                    h,
                    gfx.config.present_mode,
                    p.5 / n,
                    p.2 / n,
                    p.3 / n,
                    p.4 / n
                );
                *p = (Instant::now(), 0, 0.0, 0.0, 0.0, 0.0);
            }
        }
    }
}

impl<G: Game> ApplicationHandler for Runner<G> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_none() {
            if let Err(e) = self.init_gfx(el) {
                self.error = Some(e.to_string());
                el.exit();
                return;
            }
            // draw the first frame, then show the window with it in
            self.redraw(el);
            if let Some(gfx) = &self.gfx {
                gfx.window.set_visible(true);
                gfx.window.request_redraw();
            }
            eprintln!("[stickfell] startup: first frame shown {:.0} ms after launch", self.start.elapsed().as_secs_f64() * 1e3);
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    if size.width > 0 && size.height > 0 {
                        gfx.config.width = size.width;
                        gfx.config.height = size.height;
                        gfx.surface.configure(&gfx.device, &gfx.config);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed if !event.repeat => self.input.press(code),
                        ElementState::Released => self.input.release(code),
                        _ => {}
                    }
                }
                if event.state == ElementState::Pressed {
                    if let Some(t) = &event.text {
                        self.input.text.extend(t.chars().filter(|c| !c.is_control()));
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse = Vec2::new(position.x as f32, position.y as f32);
                self.trail.push_back((Instant::now(), self.input.mouse));
                if self.trail.len() > TRAIL_MAX {
                    self.trail.pop_front();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let idx = match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    _ => return,
                };
                let down = state == ElementState::Pressed;
                if down && !self.input.mouse_held[idx] {
                    self.input.mouse_pressed[idx] = true;
                }
                self.input.mouse_held[idx] = down;
            }
            WindowEvent::Focused(false) => {
                self.input.held.clear();
                self.input.mouse_held = [false; 3];
                // the OS lets go of the cursor; take it back on refocus
                self.focused = false;
                self.cursor_grabbed = false;
            }
            WindowEvent::Focused(true) => {
                self.focused = true;
            }
            WindowEvent::RedrawRequested => self.redraw(el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(gfx) = &self.gfx {
            gfx.window.request_redraw();
        }
    }
}

/// Opens a window and runs the game until it exits.
pub fn run<G: Game>(cfg: AppConfig, game: G) -> Result<(), Box<dyn Error>> {
    let el = EventLoop::new()?;
    el.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let mut runner = Runner {
        cfg,
        game,
        gfx: None,
        input: Input::default(),
        last: now,
        acc: 0.0,
        start: now,
        frame: Frame::default(),
        error: None,
        cursor_shown: true,
        cursor_grabbed: false,
        focused: true,
        blank_cursor: None,
        cursor_icon: CursorIcon::Default,
        trail: VecDeque::new(),
        profile: std::env::var_os("STICKFELL_PROFILE").map(|_| (now, 0, 0.0, 0.0, 0.0, 0.0)),
    };
    el.run_app(&mut runner)?;
    match runner.error {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}

/// Times starting up headless: making the renderer, then the first frames
/// (the game's draw, and the GPU finishing each), to find startup hitches.
pub fn time_startup<G: Game>(game: &mut G, size: (u32, u32), frames: u32) -> Result<(), Box<dyn Error>> {
    let t = Instant::now();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    println!("device {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let mut renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    println!("renderer {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("startup timing"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let input = Input { screen: Vec2::new(size.0 as f32, size.1 as f32), ..Default::default() };
    for i in 0..frames {
        let t = Instant::now();
        game.tick(&input);
        let mut frame = Frame::default();
        frame.time = i as f32 / 60.0;
        game.draw(&mut frame, Vec2::new(size.0 as f32, size.1 as f32), 1.0, 1.0 / 60.0);
        let t_game = t.elapsed().as_secs_f64();
        let t = Instant::now();
        renderer.render(&frame, &view, size.0, size.1);
        device.poll(wgpu::PollType::wait_indefinitely())?;
        println!("frame {i}: game {:.1} ms, render {:.1} ms", t_game * 1e3, t.elapsed().as_secs_f64() * 1e3);
    }
    Ok(())
}

/// Ticks `game` `ticks` times, then renders its frame offscreen `n` times at
/// `size` and prints the average GPU time per frame (render profiling).
pub fn bench_headless<G: Game>(game: &mut G, ticks: u32, size: (u32, u32), n: u32) -> Result<(), Box<dyn Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let mut renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let input = Input { screen: Vec2::new(size.0 as f32, size.1 as f32), ..Default::default() };
    for _ in 0..ticks {
        game.tick(&input);
    }
    let mut frame = Frame::default();
    frame.time = ticks as f32 / 60.0;
    game.draw(&mut frame, Vec2::new(size.0 as f32, size.1 as f32), 1.0, 1.0);
    let secs = renderer.bench(&frame, size.0, size.1, n);
    let (lights, casters) = renderer.last_stats();
    println!(
        "{}x{}: {:.3} ms/frame ({:.0} fps) | {} shapes, {} lights, {} occluders, {} shadow casters",
        size.0,
        size.1,
        secs * 1000.0,
        1.0 / secs,
        frame.counts().0,
        lights,
        frame.counts().1,
        casters
    );
    Ok(())
}

/// Ticks `game` `ticks` times with no input, renders one frame offscreen and
/// writes it to `out` as PNG. Needs a GPU but no window.
pub fn capture_headless<G: Game>(game: &mut G, ticks: u32, size: (u32, u32), out: &Path) -> Result<(), Box<dyn Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let mut renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let input = Input { screen: Vec2::new(size.0 as f32, size.1 as f32), ..Default::default() };
    for _ in 0..ticks {
        game.tick(&input);
    }
    let mut frame = Frame::default();
    frame.time = ticks as f32 / 60.0;
    game.draw(&mut frame, Vec2::new(size.0 as f32, size.1 as f32), 1.0, 1.0);
    let pixels = renderer.capture(&frame, size.0, size.1);
    let file = std::io::BufWriter::new(std::fs::File::create(out)?);
    let mut enc = png::Encoder::new(file, size.0, size.1);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&pixels)?;
    Ok(())
}
