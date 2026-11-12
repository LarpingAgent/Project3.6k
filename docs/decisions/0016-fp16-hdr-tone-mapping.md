# Decision 0016: FP16 HDR scene target and ACES presentation pass

## Context

The PBR sun can produce radiance above display white. Writing these values directly into an LDR swapchain clips highlights, and mixing display conversion into the material shader prevents a clear render/presentation boundary.

## Decision

- Render the scene into one per-swapchain-image `R16G16B16A16_SFLOAT` color target with a matching depth attachment.
- Require the selected device format to support optimal-tiling color-attachment rendering, sampling, and linear filtering; fail initialization with a typed error otherwise.
- End the scene render pass in shader-read-only layout, then sample that image in a fullscreen triangle pass.
- Apply a fitted ACES tone map in the presentation shader. Let sRGB swapchain attachments encode linear output; apply a simple gamma conversion only for UNORM swapchains.
- Keep tone-mapping resources, descriptors, render pass, and pipeline in a small Vulkan-owned RAII module. Resize tears down both passes only after device idle.
- Preserve the existing opt-in GPU readback path by copying the final tone-mapped swapchain image.

## Consequences

The renderer now has a true FP16 scene buffer with values above 1.0 and an independent display transform. The HDR allocation scales with the swapchain image count and resolution (8 bytes per pixel per target), so high-resolution memory cost must be measured. The ACES pass is a compact fullscreen draw. The scene still lacks shadow maps, image-based lighting, and atmospheric fog; this decision does not claim those visual features.

The validation run and captured output are recorded in `docs/rendering/vulkan-pipeline.md` and `artifacts/visual-verification/hdr-stage-1.png`.
