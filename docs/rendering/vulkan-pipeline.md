# Vulkan presentation pipeline

## Current initialization

1. Dynamically load the system Vulkan loader through `ash`.
2. Query the instance extensions required by the active `winit` display.
3. Require `VK_LAYER_KHRONOS_validation` and `VK_EXT_debug_utils` in debug builds; `--validation` can require them in release builds.
4. Create a Vulkan 1.0 instance and native window surface.
5. Select a device with `VK_KHR_swapchain` and one queue family that supports graphics and presentation. Prefer discrete, then integrated GPUs.
6. Create the logical device, swapchain, color/depth render pass, one depth image per swapchain image, framebuffers, command pool, command buffers, fences, and semaphores.
7. Compile the forest GLSL shaders with Vulkan SDK `glslc`, create a descriptor-set layout and depth-tested textured-and-lit pipeline, upload static mesh buffers and the bundled PNG texture, allocate one host-visible camera uniform and descriptor set per frame in flight, and record indexed submesh draws.

The scene embeds the workspace asset `assets/forest_noise.png` at build time and loads it through `project3-6k-assets`. The renderer-independent manager decodes to RGBA8 under a 16 MiB limit, creates an sRGB mip chain on the CPU, and tracks the CPU copies under resident-byte and asset-count budgets. Vulkan uploads every level through a temporary staging buffer into a device-local sampled image. The image transitions from undefined to transfer destination and then shader-read-only. A linear repeat sampler exposes the levels through descriptor binding 1; camera matrices remain at binding 0. The vertex color multiplies the sampled texture. GPU texture streaming and hot reload are deferred.

The static geometry builder turns the tree and ridge silhouettes into shallow prisms, emits a horizontal ground plane, and creates outward normals for prism sides and front/back faces. Each canopy front is folded into two shallow facets with normals derived from their actual triangles, producing visible directional-light contrast from the default view. The renderer applies each node's inverse-transpose normal transform when it bakes vertices for the one-time upload. The fragment shader combines the sampled, vertex-tinted albedo with a fixed world-space directional light and a 0.26 ambient floor using Lambert diffuse. Lighting values are constants in the shader, so no extra uniform or per-frame update is needed. This geometry is authored for the technical demo; it is not a general-purpose model importer or dynamic mesh path.

Core's bounded `Scene` represents the horizontal ground, colored forest panel, three trees, and distant ridge as six mesh-handle nodes with local TRS transforms. The renderer applies each node's matrix to its local vertex positions and the inverse-transpose normal matrix to its normals before the device-local upload. Node insertion order maps to indexed draw ranges, keeping the ridge last while allowing depth testing to determine visibility. This scene is static; changing a `Scene` transform after upload does not update the GPU mesh yet.

FIFO presentation is used because it is guaranteed by Vulkan and provides paced presentation. Surface formats prefer BGRA8 sRGB when available, then use the first supported format. The initial pipeline does not depend on Vulkan 1.3 dynamic rendering or other optional features.

## Frame submission

```mermaid
sequenceDiagram
  participant CPU as Platform / CPU
  participant GPU as Graphics queue
  participant WSI as Presentation engine
  CPU->>CPU: Wait for current frame fence
  CPU->>CPU: Update this frame's camera uniform
  CPU->>WSI: Acquire image (image-available semaphore)
  CPU->>CPU: Wait for prior fence owning this image
  CPU->>GPU: Submit forest pass with matching descriptor set
  Note over GPU: Wait image-available; signal image render-finished
  GPU->>WSI: Present acquired image
  Note over WSI: Wait image-specific render-finished semaphore
```

There are two CPU frames in flight. Per-image fences prevent a swapchain image from being recorded while its previous submission is pending. Render-finished semaphores are indexed by swapchain image so they are not reused before presentation has consumed the prior wait operation for that image. The renderer updates a frame's camera uniform only after its fence signals and reports whether `queue_submit` succeeded separately from whether swapchain recreation is needed, so automated frame counts do not count acquire failures as rendered frames.

## Resize and teardown

Zero-sized windows suspend drawing. A nonzero resize, out-of-date acquisition, or suboptimal presentation triggers a device-idle wait, destruction of swapchain-dependent resources, and recreation. Framebuffers are destroyed before the render pass, and image views before the swapchain. The renderer is dropped before its native window. `--resize-stress` requests two sizes, minimizes, and restores the window; the run fails unless it observes resize events including a zero extent.

For repeatable lifecycle checks, the app accepts `--run-frames N` (positive N; exits after N successful submissions), `--auto-close` (exits immediately after renderer initialization), and `--validation` (requires validation layers). All options go through the normal event-loop exit and shutdown code. Debug builds always enable validation. The callback counts and logs every warning/error; after renderer and window destruction the app exits nonzero if any occurred. Shutdown logs the submitted frame count.

The render pass clears the per-image depth attachment to 1.0 and uses a pipeline with depth testing and writes enabled (`LESS`). The depth format is selected from the physical device's optimal-tiling support. See [Decision 0007](../decisions/0007-depth-attachments.md) for the per-image lifetime rationale. The sampled texture uses `R8G8B8A8_SRGB` when the device reports sampled-image and linear-filter support.

Run `scripts/validation-smoke.ps1` to execute the 1, 60, and 600 frame runs and a resize-stress run. Logs go under `target/validation-runs/`. The scene has a horizontal ground plane and volumetric low-poly silhouette prisms, with separate indexed ranges at different world-space depths and one depth attachment per swapchain image. Automated validation does not prove the final pixels; ground contact, texture appearance, and directional shading still require the manual checklist. Offscreen image readback remains a future automated renderer gate.

## Next rendering steps

The shader build path invokes `glslc` from `VULKAN_SDK` and writes SPIR-V to Cargo's `OUT_DIR`; no compiler dependency or committed shader binaries are needed. Static mesh data is copied through temporary host-visible staging buffers into device-local vertex/index and texture images, with explicit transfer-to-input/shader-read barriers. `glam` supplies camera math in core; Vulkan receives only a column-major matrix array. Depth attachment ownership and format selection are described in [Decision 0007](../decisions/0007-depth-attachments.md), static low-poly shading in [Decision 0009](../decisions/0009-basic-lighting.md), and the authored spatial demo geometry in [Decision 0013](../decisions/0013-spatial-low-poly-forest.md). Shadow maps, PBR, and large-world systems remain later extensions.
