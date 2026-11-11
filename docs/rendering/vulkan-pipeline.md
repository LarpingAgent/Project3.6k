# Vulkan presentation pipeline

## Current initialization

1. Dynamically load the system Vulkan loader through `ash`.
2. Query the instance extensions required by the active `winit` display.
3. Require `VK_LAYER_KHRONOS_validation` and `VK_EXT_debug_utils` in debug builds; `--validation` can require them in release builds.
4. Create a Vulkan 1.0 instance and native window surface.
5. Select a device with `VK_KHR_swapchain` and one queue family that supports graphics and presentation. Prefer discrete, then integrated GPUs.
6. Create the logical device, swapchain, image views, color-only render pass, framebuffers, command pool, command buffers, fences, and semaphores.
7. Compile the forest GLSL shaders with Vulkan SDK `glslc`, create the vertex-color pipeline, upload static mesh buffers, and record indexed draws.

FIFO presentation is used because it is guaranteed by Vulkan and provides paced presentation. Surface formats prefer BGRA8 sRGB when available, then use the first supported format. The initial pipeline does not depend on Vulkan 1.3 dynamic rendering or other optional features.

## Frame submission

```mermaid
sequenceDiagram
  participant CPU as Platform / CPU
  participant GPU as Graphics queue
  participant WSI as Presentation engine
  CPU->>CPU: Wait for current frame fence
  CPU->>WSI: Acquire image (image-available semaphore)
  CPU->>CPU: Wait for prior fence owning this image
  CPU->>GPU: Submit clear render pass
  Note over GPU: Wait image-available; signal image render-finished
  GPU->>WSI: Present acquired image
  Note over WSI: Wait image-specific render-finished semaphore
```

There are two CPU frames in flight. Per-image fences prevent a swapchain image from being recorded while its previous submission is pending. Render-finished semaphores are indexed by swapchain image so they are not reused before presentation has consumed the prior wait operation for that image. The renderer reports whether `queue_submit` succeeded separately from whether swapchain recreation is needed, so automated frame counts do not count acquire failures as rendered frames.

## Resize and teardown

Zero-sized windows suspend drawing. A nonzero resize, out-of-date acquisition, or suboptimal presentation triggers a device-idle wait, destruction of swapchain-dependent resources, and recreation. Framebuffers are destroyed before the render pass, and image views before the swapchain. The renderer is dropped before its native window. `--resize-stress` requests two sizes, minimizes, and restores the window; the run fails unless it observes resize events including a zero extent.

For repeatable lifecycle checks, the app accepts `--run-frames N` (positive N; exits after N successful submissions), `--auto-close` (exits immediately after renderer initialization), and `--validation` (requires validation layers). All options go through the normal event-loop exit and shutdown code. Debug builds always enable validation. The callback counts and logs every warning/error; after renderer and window destruction the app exits nonzero if any occurred. Shutdown logs the submitted frame count.

Run `scripts/validation-smoke.ps1` to execute the 1, 60, and 600 frame runs and a resize-stress run. Logs go under `target/validation-runs/`. The current forest mesh is static clip-space geometry; perspective projection, depth, textures, and lighting remain future work. Add offscreen image readback to the automated renderer gate now that geometry exists.

## Next rendering steps

The shader build path invokes `glslc` from `VULKAN_SDK` and writes SPIR-V to Cargo's `OUT_DIR`; no compiler dependency or committed shader binaries are needed. The next rendering steps are camera uniforms, perspective projection, depth testing, textured geometry, and basic lighting. GPU suballocation and staging uploads will be evaluated as mesh/resource requirements grow. Shadow maps, PBR, and large-world systems remain later extensions.
