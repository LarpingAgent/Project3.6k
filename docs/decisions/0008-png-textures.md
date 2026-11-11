# Decision 0008: use a bundled PNG texture with a CPU mip chain

## Context

The forest geometry needs a sampled texture before a general asset system exists. The renderer currently loads one static image during scene initialization, and the workspace targets Rust 1.85 with Vulkan 1.0.

## Options considered

- Add a general image and asset framework before a concrete loading case: broader functionality, but premature APIs and dependencies.
- Decode PNG with a focused crate, embed the small demo asset, and upload it as a sampled Vulkan image: covers the current scene without runtime file lookup or a GPU blit requirement.
- Store preconverted raw pixels: removes runtime decoding but loses a portable, editable source asset and requires a project-specific conversion tool.

## Decision

Use `png` 0.18.1, which supports Rust versions below the workspace's 1.85 minimum. Keep the editable `assets/forest_noise.png` source in the workspace-level asset directory and embed it with `include_bytes!` so initialization does not depend on the process working directory. Apply a 16 MiB decode limit, normalize supported PNG color types to RGBA8, and reject devices that cannot sample and linearly filter `R8G8B8A8_SRGB`.

Generate each mip level on the CPU. Decode color channels to linear light before averaging and encode them back to sRGB; average alpha linearly. Upload all levels with one temporary host-visible staging buffer into a device-local image, then transition it to shader-read-only layout. A linear repeat sampler supplies the mip chain to the forest fragment shader. The vertex color multiplies the sampled texel.

## Consequences

The static scene has one portable bundled texture and does not require runtime file paths or optional GPU blit support. CPU mip generation costs initialization time and temporary memory, which is acceptable for this small static asset. Decoded pixels and mip levels live in the renderer-independent `AssetManager` during loading, then are released after the static GPU upload. The GPU image remains owned by `ForestScene`. Streamed GPU textures, alternate formats, anisotropic filtering, and asset hot reload remain future work. Interactive appearance and filtering still require the manual checklist.
