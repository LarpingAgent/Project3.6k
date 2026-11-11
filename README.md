# Project3.6k

Project3.6k is a custom Rust engine foundation for a future first-person forest sandbox. It uses direct Vulkan calls through `ash`; it does not use a game engine or a graphics abstraction layer.

## Current foundation

- Cargo workspace with focused `core`, `platform`, `vulkan`, and `app` crates.
- Native `winit` window and application lifecycle, paced at approximately 60 redraw requests per second.
- Physical keyboard state and Escape-to-close input handling.
- Vulkan instance and device selection, native surface, swapchain, render pass, command buffers, fences, and semaphores.
- Clear-only forest-colored frames with resize and out-of-date swapchain handling.
- Optional `VK_LAYER_KHRONOS_validation` when the layer and debug-utils extension are installed.
- Monotonic frame timing and structured `tracing` logs.

This is the application and Vulkan submission foundation. It does not yet render geometry, compile shaders, move a camera, or implement gameplay.

## Requirements

- Stable Rust 1.85 or newer with the Windows MSVC target.
- Visual Studio 2022 or Build Tools with the **Desktop development with C++** workload, including the MSVC linker and Windows SDK.
- A Vulkan-capable graphics driver. The Vulkan SDK is optional for the current runtime path; install it to get the Khronos validation layer and shader compiler tools.

The renderer dynamically loads the system Vulkan loader, so this phase does not need Vulkan SDK headers or link-time Vulkan libraries.

## Build and run

```powershell
cargo run -p project3-6k-app
```

Press **Escape** or close the window to exit. Set `RUST_LOG=debug` to see keyboard events and periodic frame timing; `RUST_LOG=project3_6k_vulkan=trace` can be used when additional renderer tracing is added.

For automated lifecycle checks, `cargo run -p project3-6k-app -- --run-frames 60` exits after 60 frames have been submitted successfully. `cargo run -p project3-6k-app -- --auto-close` exits after Vulkan initialization. Both options use the regular shutdown path.

## Workspace map

| Crate | Responsibility |
|---|---|
| `project3-6k-core` | Platform-neutral frame timing primitives. |
| `project3-6k-platform` | `winit` event loop, window lifecycle, and input state. |
| `project3-6k-vulkan` | Explicit Vulkan instance, surface, device, swapchain, synchronization, and clear-frame submission. |
| `project3-6k-app` | Executable entry point and logging setup. |

See [ARCHITECTURE.md](ARCHITECTURE.md) and the [architecture docs](docs/architecture/overview.md) for ownership and extension decisions.
