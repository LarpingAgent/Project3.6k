# Project3.6k

Project3.6k is a custom Rust engine foundation for a future first-person forest sandbox. It uses direct Vulkan calls through `ash`; it does not use a game engine or a graphics abstraction layer.

## Current foundation

- Cargo workspace with focused `assets`, `core`, `platform`, `vulkan`, and `app` crates.
- Game texture sources live under the workspace-level `assets/` directory.
- Renderer-independent raster texture decoding with dimension and byte limits.
- Static glTF 2.0/GLB CPU import with mesh instances, generated normals/tangents, PBR factors, and base-color, metallic-roughness, normal, occlusion, and emissive map references.
- Bounded CPU model cache and a single-thread asynchronous loader with back-pressure; Vulkan resources remain outside the `assets` crate.
- Bounded mesh-instance scene data with scale/rotation/translation transforms, baked into the static forest mesh at initialization.
- Native `winit` window and application lifecycle, paced at approximately 60 redraw requests per second.
- Physical keyboard state, raw mouse input, cursor capture/release, and first-person WASD camera controls.
- Vulkan instance and device selection, native surface, swapchain, render pass, GLSL graphics pipeline, vertex/index buffers, command buffers, fences, semaphores, and swapchain recreation.
- Debug builds require `VK_LAYER_KHRONOS_validation`; the callback counts and logs warnings/errors and makes the process fail if any occur.
- Monotonic frame timing plus a fixed-size rolling profiler for FPS, average/p95/max frame interval, and renderer-call CPU time.

The current renderer still builds its original static spatial forest from authored low-poly geometry and an embedded repeating sRGB noise texture. The new CPU importer and PBR material data are not yet connected to Vulkan draws; GPU asset ownership, HDR/tone mapping, shadow maps, and the external-asset reference scene are subsequent verified steps. Interactive visual quality is still unverified; gameplay is outside this cycle.

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

Left-click to capture the mouse, use **W/A/S/D** to move and **Space/Shift** to rise or descend. Press **Escape** to release the cursor, then press it again or close the window to exit. The title bar refreshes every 120 frames with estimated FPS, average/p95 frame interval, mean renderer-call CPU time, and camera position. Set `RUST_LOG=debug` to log the same rolling profile with input events. Renderer-call CPU time includes CPU-side waiting/submission/presentation calls; it is not GPU execution time.

For automated lifecycle checks, `cargo run -p project3-6k-app -- --run-frames 60 --validation` exits after 60 successful submissions through the regular shutdown path. `--auto-close` exits after Vulkan initialization. `--resize-stress` requires `--run-frames N`; it changes size, minimizes, and restores the window, and fails unless a zero-extent event is observed.

Run the full Phase 2 validation smoke suite from PowerShell with `scripts/validation-smoke.ps1`. It runs 1, 60, and 600 frame submissions, followed by resize stress, and saves logs under `target/validation-runs/`.

## Workspace map

| Crate | Responsibility |
|---|---|
| `project3-6k-assets` | Bounded raster decoding, glTF/GLB CPU import, PBR material data, model/texture caches, and asynchronous CPU loading. |
| `project3-6k-core` | Platform-neutral frame timing/profiling, camera math, bounded scene nodes, and transforms (`glam`). |
| `project3-6k-platform` | `winit` event loop, window lifecycle, keyboard/mouse input, and camera controls. |
| `project3-6k-vulkan` | Explicit Vulkan instance, surface, device, swapchain, color/depth attachments, synchronization, validation monitoring, per-frame camera uniforms, sampled textures, ambient/directional lighting, and indexed forest rendering. |
| `project3-6k-app` | Executable entry point and logging setup. |

See [ARCHITECTURE.md](ARCHITECTURE.md) and the [architecture docs](docs/architecture/overview.md) for ownership and extension decisions.
