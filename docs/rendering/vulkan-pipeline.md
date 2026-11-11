# Vulkan presentation pipeline

## Current initialization

1. Dynamically load the system Vulkan loader through `ash`.
2. Query the instance extensions required by the active `winit` display.
3. Enable `VK_LAYER_KHRONOS_validation` and `VK_EXT_debug_utils` in debug builds only when both are installed.
4. Create a Vulkan 1.0 instance and native window surface.
5. Select a device with `VK_KHR_swapchain` and one queue family that supports graphics and presentation. Prefer discrete, then integrated GPUs.
6. Create the logical device, swapchain, image views, color-only render pass, framebuffers, command pool, command buffers, fences, and semaphores.

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

Zero-sized windows suspend drawing. A nonzero resize, out-of-date acquisition, or suboptimal presentation triggers a device-idle wait, destruction of swapchain-dependent resources, and recreation. Framebuffers are destroyed before the render pass, and image views before the swapchain. The renderer is dropped before its native window.

For repeatable lifecycle checks, the app accepts `--run-frames N` (positive N; exits after N successful submissions) and `--auto-close` (exits immediately after renderer initialization). Both go through the normal event-loop exit and shutdown code. Shutdown logs the submitted frame count after renderer and window destruction.

## Next rendering steps

Add a shader build path, graphics pipeline, vertex/index buffers, depth attachment, camera uniforms, and textured geometry. Introduce GPU suballocation only when actual resource needs and alignment requirements are known. Shadow maps, PBR, and large-world systems remain later extensions.
