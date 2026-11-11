# Project3.6k

Project3.6k is a custom Rust engine foundation for a future first-person forest sandbox. It uses direct Vulkan calls through `ash`; it does not use a game engine or a graphics abstraction layer.

## Current foundation

- Cargo workspace with focused `core`, `platform`, `vulkan`, and `app` crates.
- Native `winit` window and application lifecycle, paced at approximately 60 redraw requests per second.
- Physical keyboard state, raw mouse input, cursor capture/release, and first-person WASD camera controls.
- Vulkan instance and device selection, native surface, swapchain, render pass, GLSL graphics pipeline, vertex/index buffers, command buffers, fences, semaphores, and swapchain recreation.
- Debug builds require `VK_LAYER_KHRONOS_validation`; the callback counts and logs warnings/errors and makes the process fail if any occur.
- Monotonic frame timing and structured `tracing` logs.

The renderer currently shows a static vertex-colored planar forest composition using indexed triangles, transformed by a first-person perspective camera. Depth testing, textures, lighting, and gameplay are still future work.

## Requirements

- Stable Rust 1.85 or newer with the Windows MSVC target.
- Visual Studio 2022 or Build Tools with the **Desktop development with C++** workload, including the MSVC linker and Windows SDK.
- A Vulkan-capable graphics driver.
- The Vulkan SDK with `VK_LAYER_KHRONOS_validation` for debug builds and automated validation runs. Release builds can run without it unless `--validation` is passed.
- The Vulkan SDK `glslc` compiler is used by the Vulkan crate build script to compile GLSL into ignored `OUT_DIR` SPIR-V files.

The renderer dynamically loads the system Vulkan loader, so this phase does not need Vulkan SDK headers or link-time Vulkan libraries.

## Build and run

```powershell
cargo run -p project3-6k-app
```

Left-click to capture the mouse, use **W/A/S/D** to move and **Space/Shift** to rise or descend. Press **Escape** to release the cursor, then press it again or close the window to exit. Set `RUST_LOG=debug` to see keyboard events and periodic frame timing; `RUST_LOG=project3_6k_vulkan=trace` can be used when additional renderer tracing is added.

For automated lifecycle checks, `cargo run -p project3-6k-app -- --run-frames 60 --validation` exits after 60 successful submissions through the regular shutdown path. `--auto-close` exits after Vulkan initialization. `--resize-stress` requires `--run-frames N`; it changes size, minimizes, and restores the window, and fails unless a zero-extent event is observed.

Run the full Phase 2 validation smoke suite from PowerShell with `scripts/validation-smoke.ps1`. It runs 1, 60, and 600 frame submissions, followed by resize stress, and saves logs under `target/validation-runs/`.

## Workspace map

| Crate | Responsibility |
|---|---|
| `project3-6k-core` | Platform-neutral frame timing and camera math (`glam`). |
| `project3-6k-platform` | `winit` event loop, window lifecycle, keyboard/mouse input, and camera controls. |
| `project3-6k-vulkan` | Explicit Vulkan instance, surface, device, swapchain, synchronization, validation monitoring, per-frame camera uniforms, and indexed forest rendering. |
| `project3-6k-app` | Executable entry point and logging setup. |

See [ARCHITECTURE.md](ARCHITECTURE.md) and the [architecture docs](docs/architecture/overview.md) for ownership and extension decisions.
