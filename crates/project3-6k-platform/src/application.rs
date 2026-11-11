use std::time::{Duration, Instant};

use project3_6k_core::{Camera, FrameClock, FrameProfiler};
use project3_6k_vulkan::{VulkanRenderer, VulkanRendererError};
use thiserror::Error;
use tracing::{debug, error, info, warn};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    error::{EventLoopError, OsError},
    event::{DeviceEvent, ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

use crate::InputState;

const MOUSE_SENSITIVITY: f32 = 0.0025;

const TARGET_FRAME_DURATION: Duration = Duration::from_nanos(16_666_667);

/// Errors produced while creating the native application.
#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("could not create the event loop: {0}")]
    EventLoop(#[from] EventLoopError),
    #[error("could not create the window: {0}")]
    Window(#[from] OsError),
    #[error("could not initialize the renderer: {0}")]
    Renderer(#[from] VulkanRendererError),
    #[error("Vulkan validation reported {warnings} warning(s) and {errors} error(s)")]
    ValidationMessages { warnings: usize, errors: usize },
    #[error(
        "resize stress did not exercise the expected extents (resize events: {resize_events}, zero-extent events: {zero_extent_events})"
    )]
    ResizeStressIncomplete {
        resize_events: u64,
        zero_extent_events: u64,
    },
}

/// Controls automatic exit for repeatable renderer lifecycle checks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RunOptions {
    /// Exit after this many frames have been submitted successfully.
    pub run_frames: Option<u64>,
    /// Exit after the window and renderer have initialized.
    pub auto_close: bool,
    /// Require validation even in release builds. Debug builds always require it.
    pub validation: bool,
    /// Programmatically resize, minimize, and restore the window during the run.
    pub resize_stress: bool,
}

/// Starts the Project3.6k application and runs its native event loop.
pub fn run() -> Result<(), ApplicationError> {
    run_with_options(RunOptions::default())
}

/// Starts the application with deterministic lifecycle options for automated runs.
pub fn run_with_options(options: RunOptions) -> Result<(), ApplicationError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::WaitUntil(
        Instant::now() + TARGET_FRAME_DURATION,
    ));
    let mut application = EngineApplication::default();
    application.options = options;
    event_loop.run_app(&mut application)?;
    let failure = application.failure.take();
    let validation_monitor = application
        .renderer
        .as_ref()
        .and_then(VulkanRenderer::validation_monitor);
    let resize_events = application.resize_events;
    let zero_extent_events = application.zero_extent_events;
    drop(application);

    if let Some(error) = failure {
        return Err(error);
    }
    if let Some(report) = validation_monitor.map(|monitor| monitor.report()) {
        if report.has_issues() {
            return Err(ApplicationError::ValidationMessages {
                warnings: report.warnings,
                errors: report.errors,
            });
        }
        info!(warnings = 0, errors = 0, "Vulkan validation clean");
    }
    if options.resize_stress && (resize_events < 3 || zero_extent_events == 0) {
        return Err(ApplicationError::ResizeStressIncomplete {
            resize_events,
            zero_extent_events,
        });
    }
    if options.resize_stress {
        info!(resize_events, zero_extent_events, "resize stress completed");
    }
    Ok(())
}

#[derive(Default)]
struct EngineApplication {
    options: RunOptions,
    window: Option<Window>,
    renderer: Option<VulkanRenderer>,
    input: InputState,
    camera: Camera,
    cursor_captured: bool,
    clock: Option<FrameClock>,
    profiler: FrameProfiler,
    failure: Option<ApplicationError>,
    frames_rendered: u64,
    resize_events: u64,
    zero_extent_events: u64,
    resize_stress_started: Option<Instant>,
    resize_stress_stage: u8,
}

impl ApplicationHandler for EngineApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() || self.failure.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("Project3.6k — Engine Foundation")
            .with_inner_size(PhysicalSize::new(1280, 720));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => window,
            Err(error) => {
                error!(%error, "window creation failed");
                self.failure = Some(ApplicationError::Window(error));
                event_loop.exit();
                return;
            }
        };
        info!("native window created");

        match VulkanRenderer::new_with_validation(&window, self.options.validation) {
            Ok(renderer) => {
                self.clock = Some(FrameClock::default());
                self.renderer = Some(renderer);
                self.window = Some(window);
                info!(
                    "application ready; click to capture mouse, WASD to move, Space/Shift to rise or descend, Escape to release or close"
                );
                if self.options.auto_close {
                    info!("auto-close requested after renderer initialization");
                    event_loop.exit();
                }
            }
            Err(error) => {
                error!(%error, "Vulkan initialization failed");
                self.failure = Some(ApplicationError::Renderer(error));
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(Window::id) != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.resize_events = self.resize_events.saturating_add(1);
                if size.width == 0 || size.height == 0 {
                    self.zero_extent_events = self.zero_extent_events.saturating_add(1);
                    info!(
                        width = size.width,
                        height = size.height,
                        "window entered zero extent"
                    );
                }
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => {
                self.input.clear();
                self.set_cursor_capture(false);
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } if !self.cursor_captured => self.set_cursor_capture(true),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    let is_down = event.state == ElementState::Pressed;
                    self.input.set_key_down(key, is_down);
                    if is_down && key == KeyCode::Escape {
                        if self.cursor_captured {
                            self.set_cursor_capture(false);
                            info!("mouse released; press Escape again to close");
                        } else {
                            event_loop.exit();
                        }
                    } else if is_down && !event.repeat {
                        debug!(?key, "keyboard input received");
                    }
                }
            }
            WindowEvent::RedrawRequested => self.render_frame(event_loop),
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if self.cursor_captured {
            if let DeviceEvent::MouseMotion { delta } = event {
                self.input.add_mouse_motion(delta);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.advance_resize_stress();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + TARGET_FRAME_DURATION,
            ));
        }
    }
}

impl EngineApplication {
    fn set_cursor_capture(&mut self, captured: bool) {
        let Some(window) = self.window.as_ref() else {
            self.cursor_captured = false;
            return;
        };

        if captured {
            let grabbed = match window.set_cursor_grab(CursorGrabMode::Locked) {
                Ok(()) => true,
                Err(locked_error) => match window.set_cursor_grab(CursorGrabMode::Confined) {
                    Ok(()) => {
                        info!("locked cursor unavailable; using confined cursor");
                        true
                    }
                    Err(confined_error) => {
                        warn!(%locked_error, %confined_error, "could not capture cursor");
                        false
                    }
                },
            };
            self.cursor_captured = grabbed;
            window.set_cursor_visible(!grabbed);
        } else {
            if let Err(error) = window.set_cursor_grab(CursorGrabMode::None) {
                warn!(%error, "could not release cursor grab");
            }
            window.set_cursor_visible(true);
            self.cursor_captured = false;
        }
    }

    fn advance_resize_stress(&mut self) {
        if !self.options.resize_stress || self.resize_stress_stage >= 4 {
            return;
        }
        let started = *self.resize_stress_started.get_or_insert_with(Instant::now);
        let deadlines = [
            Duration::from_millis(250),
            Duration::from_millis(500),
            Duration::from_millis(750),
            Duration::from_millis(1_050),
        ];
        let stage = self.resize_stress_stage as usize;
        if started.elapsed() < deadlines[stage] {
            return;
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        match stage {
            0 => {
                let _ = window.request_inner_size(PhysicalSize::new(1024, 640));
                info!(width = 1024, height = 640, "resize stress requested size");
            }
            1 => {
                let _ = window.request_inner_size(PhysicalSize::new(720, 480));
                info!(width = 720, height = 480, "resize stress requested size");
            }
            2 => {
                window.set_minimized(true);
                info!("resize stress minimized window");
            }
            3 => {
                window.set_minimized(false);
                let _ = window.request_inner_size(PhysicalSize::new(1280, 720));
                info!("resize stress restored window");
            }
            _ => unreachable!(),
        }
        self.resize_stress_stage += 1;
    }

    fn render_frame(&mut self, event_loop: &ActiveEventLoop) {
        let frame_time = self.clock.as_mut().map(FrameClock::tick);
        let delta_seconds = frame_time
            .map(|time| time.delta.min(Duration::from_millis(100)).as_secs_f32())
            .unwrap_or(0.0);
        let [mouse_x, mouse_y] = self.input.take_mouse_delta();
        if self.cursor_captured {
            self.camera.rotate(
                (mouse_x as f32) * MOUSE_SENSITIVITY,
                (mouse_y as f32) * MOUSE_SENSITIVITY,
            );
        }
        let forward = self.input.is_key_down(KeyCode::KeyW) as u8 as f32
            - self.input.is_key_down(KeyCode::KeyS) as u8 as f32;
        let right = self.input.is_key_down(KeyCode::KeyD) as u8 as f32
            - self.input.is_key_down(KeyCode::KeyA) as u8 as f32;
        let vertical = self.input.is_key_down(KeyCode::Space) as u8 as f32
            - (self.input.is_key_down(KeyCode::ShiftLeft)
                || self.input.is_key_down(KeyCode::ShiftRight)) as u8 as f32;
        self.camera
            .move_local(forward, right, vertical, delta_seconds);
        let aspect_ratio = self
            .window
            .as_ref()
            .map(Window::inner_size)
            .map(|size| size.width as f32 / size.height.max(1) as f32)
            .unwrap_or(1.0);
        let view_projection = self.camera.view_projection(aspect_ratio).to_cols_array_2d();

        let (Some(window), Some(renderer)) = (self.window.as_ref(), self.renderer.as_mut()) else {
            return;
        };

        let renderer_started = Instant::now();
        let draw_result = renderer.draw(window, view_projection);
        let renderer_cpu = renderer_started.elapsed();
        let submitted = match draw_result {
            Ok(submitted) => submitted,
            Err(error) => {
                error!(%error, "frame rendering failed");
                self.failure = Some(ApplicationError::Renderer(error));
                event_loop.exit();
                return;
            }
        };

        if submitted {
            self.frames_rendered += 1;
            if self.options.run_frames == Some(self.frames_rendered) {
                info!(
                    frames = self.frames_rendered,
                    "requested frame count reached"
                );
                event_loop.exit();
            }
        }

        if let Some(stats) =
            frame_time.and_then(|frame_time| self.profiler.record(frame_time.delta, renderer_cpu))
        {
            debug!(
                samples = stats.sample_count,
                fps = stats.frames_per_second,
                average_frame_ms = stats.average_frame_ms,
                p95_frame_ms = stats.p95_frame_ms,
                maximum_frame_ms = stats.maximum_frame_ms,
                average_renderer_cpu_ms = stats.average_renderer_cpu_ms,
                "rolling frame profile"
            );
            if let Some(window) = self.window.as_ref() {
                let position = self.camera.position();
                window.set_title(&format!(
                    "Project3.6k | {:.0} FPS | {:.1} ms avg / {:.1} p95 | renderer {:.2} ms | pos {:.1}, {:.1}, {:.1}",
                    stats.frames_per_second,
                    stats.average_frame_ms,
                    stats.p95_frame_ms,
                    stats.average_renderer_cpu_ms,
                    position.x,
                    position.y,
                    position.z,
                ));
            }
        }
    }
}

impl Drop for EngineApplication {
    fn drop(&mut self) {
        if self.cursor_captured {
            self.set_cursor_capture(false);
        }
        // The Vulkan surface must be destroyed before its native window. Take the renderer
        // first so its swapchain, surface, and instance are torn down while the window lives.
        drop(self.renderer.take());
        drop(self.window.take());
        info!(
            frames_rendered = self.frames_rendered,
            "application shutdown complete"
        );
    }
}
