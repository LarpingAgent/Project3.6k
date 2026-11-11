use std::time::{Duration, Instant};

use project3_6k_core::FrameClock;
use project3_6k_vulkan::{VulkanRenderer, VulkanRendererError};
use thiserror::Error;
use tracing::{debug, error, info};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    error::{EventLoopError, OsError},
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use crate::InputState;

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
}

/// Controls automatic exit for repeatable renderer lifecycle checks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RunOptions {
    /// Exit after this many frames have been submitted successfully.
    pub run_frames: Option<u64>,
    /// Exit after the window and renderer have initialized.
    pub auto_close: bool,
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
    if let Some(error) = application.failure.take() {
        return Err(error);
    }
    Ok(())
}

#[derive(Default)]
struct EngineApplication {
    options: RunOptions,
    window: Option<Window>,
    renderer: Option<VulkanRenderer>,
    input: InputState,
    clock: Option<FrameClock>,
    failure: Option<ApplicationError>,
    frames_rendered: u64,
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

        match VulkanRenderer::new(&window) {
            Ok(renderer) => {
                self.clock = Some(FrameClock::default());
                self.renderer = Some(renderer);
                self.window = Some(window);
                info!("application foundation ready; press Escape to close");
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
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => self.input.clear(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    let is_down = event.state == ElementState::Pressed;
                    self.input.set_key_down(key, is_down);
                    if is_down && key == KeyCode::Escape {
                        event_loop.exit();
                    } else if is_down && !event.repeat {
                        debug!(?key, "keyboard input received");
                    }
                }
            }
            WindowEvent::RedrawRequested => self.render_frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + TARGET_FRAME_DURATION,
            ));
        }
    }
}

impl EngineApplication {
    fn render_frame(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(window), Some(renderer)) = (self.window.as_ref(), self.renderer.as_mut()) else {
            return;
        };

        let submitted = match renderer.draw(window) {
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

        let Some(clock) = self.clock.as_mut() else {
            return;
        };
        let frame_time = clock.tick();
        if frame_time.frame_index % 120 == 0 {
            debug!(
                frame = frame_time.frame_index,
                delta_ms = frame_time.delta.as_secs_f64() * 1000.0,
                "frame timing"
            );
        }
    }
}

impl Drop for EngineApplication {
    fn drop(&mut self) {
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
