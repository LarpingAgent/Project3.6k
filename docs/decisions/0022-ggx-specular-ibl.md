# Decision 0022: Add GGX-prefiltered specular image-based lighting

## Context

The reference renderer already used a licensed HDR environment for the visible sky and diffuse spherical-harmonic lighting. Specular IBL still sampled the ordinary box-filtered sky mips and multiplied by a fixed dielectric reflectance, which did not account for the view angle, material roughness integration, or split-sum BRDF response.

## Decision

- Generate a bounded 256×128 GGX-prefiltered equirectangular radiance chain on the CPU from the source HDR mips, using Hammersley samples and sample-footprint source LOD selection.
- Generate a 128×128 split-sum BRDF integration LUT on the CPU.
- Keep diffuse irradiance in the existing nine-coefficient spherical-harmonic representation.
- Upload the specular chain and LUT as RAII-owned FP16 Vulkan textures in `ForestScene`, alongside the source HDR sky texture.
- Sample prefiltered radiance by roughness and combine it with the LUT's scale/bias terms in the existing PBR fragment shader.
- Include all retained CPU environment vectors in the configured import memory limit; keep CPU generation independent of Vulkan.

## Consequences

The PBR path now has roughness-aware GGX specular IBL with the standard split-sum approximation. It is not path tracing and the sparse reference scene does not make the visual difference dramatic on its rough rock and foliage materials. The retained environment CPU data is 45,700,448 bytes and FP16 GPU data is 22,850,224 bytes. On the NVIDIA GeForce GTX 1660 SUPER, environment decode, prefilter, LUT generation, and upload took about 283 ms. A validation-enabled 600-frame Release run rendered at about 75.3 frames/s at the window cap; validation reported zero warnings and errors.

The inspected 1280×720 output is [forest-pine-ggx-brdf-final.png](../../artifacts/visual-verification/forest-pine-ggx-brdf-final.png). It confirms that the shader, real environment, material textures, and shadow pass render together, while also showing the remaining flat ground and sparse vegetation limits.
