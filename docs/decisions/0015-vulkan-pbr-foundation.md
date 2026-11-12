# Decision 0015: Vulkan GPU asset cache and PBR foundation

## Context

CPU-side glTF import is now present, but the existing renderer only draws its authored demo geometry and one noise texture. The next increment needs to display licensed external models while keeping CPU decoding independent of Vulkan and preserving explicit Vulkan ownership.

## Decision

- Keep file parsing, image decoding, limits, and CPU model/material data in `project3-6k-assets`.
- Keep all Vulkan buffers, images, samplers, descriptors, and GPU cache handles in `project3-6k-vulkan`.
- Upload model geometry through temporary host-visible staging buffers into device-local vertex/index buffers. Upload each mip chain into a device-local sampled image.
- Track role-specific mip generation and formats: sRGB for color/emissive, linear UNORM for metallic-roughness/AO, and vector-aware linear UNORM for normal maps.
- Bound GPU model count to 32 and estimated resident model resource bytes to 256 MiB. Use generational model handles. Resource wrappers own Vulkan handles and release them after the renderer has waited for device idle.
- Start with a single GGX PBR directional-sun shader, one scene-uniform set per frame, five sampled material bindings, opaque/mask alpha, and `TEXCOORD_0`. Return errors for blend materials or other UV sets until their render paths are implemented.
- Use real CC0 Poly Haven models and ground maps for a renderer technical scene. Keep asset author/source/license records and per-file checksums alongside the asset payloads.
- Treat the current fragment-shader Reinhard mapping as a temporary LDR display transform. Implement a true FP16 HDR target and separate tone-mapping pass as the next rendering increment; directional shadow mapping follows after HDR verification.

## Consequences

The renderer can now exercise external mesh geometry, normal maps, metallic/roughness maps, AO, and alpha-masked foliage through Vulkan. CPU and GPU ownership stay separated, and startup loads are bounded. The initial GPU cache does not yet provide background GPU uploads or streaming; it synchronously uploads after CPU worker completion and waits for the transfer queue to finish. The reference scene is visibly more material-rich than the authored baseline, but has no cast shadows, environment lighting, or atmospheric fog and is not yet a finished forest visual target.

Visual evidence is captured through bounded swapchain readback in `artifacts/visual-verification/`; each new renderer stage must be opened and compared before claiming a quality improvement.
