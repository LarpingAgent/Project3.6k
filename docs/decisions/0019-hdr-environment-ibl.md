# Decision 0019: Add HDR environment lighting to the reference scene

## Context

The PBR reference scene had a directional sun and shadow map, but its constant ambient term left shadowed foliage dark and did not respond to the surrounding environment. Increasing the constant ambient indiscriminately also reduced shadow contrast.

## Decision

- Use Poly Haven's [Forest Slope](https://polyhaven.com/a/forest_slope) 1K Radiance HDRI, licensed CC0, as the lighting source. Keep the HDR file and provenance in the repository.
- Decode bounded HDR data and generate linear-light mip levels in `project3-6k-assets`, which remains independent of Vulkan.
- Integrate Lambertian diffuse irradiance into nine real spherical-harmonic coefficients during CPU import. Upload the environment texture as FP16 through the Vulkan resource layer; keep its lifetime owned by `ForestScene`.
- Evaluate diffuse irradiance from the compact coefficients in the PBR fragment shader, and use the HDR texture's ordinary mip chain for roughness-selected specular reflections.
- Add a restrained opposite-side diffuse term to alpha-masked materials to approximate foliage light transmission.
- Keep startup loading synchronous for this single reference HDRI. The existing bounded worker continues to handle model imports; streaming and asynchronous GPU upload remain later work.

## Consequences

The scene receives environment-dependent indirect lighting without adding geometry or diffuse texture reads per fragment. The HDRI import and upload took about 35 ms at startup on the test machine; its FP16 mip payload uses 5,592,408 bytes. The 600-frame Release run without Vulkan validation stayed near the 75 Hz display cap at 74.9–77.5 FPS with p95 frame intervals of 13.8–14.3 ms.

The visual reference is [forest-pine-ibl-final.png](../../artifacts/visual-verification/forest-pine-ibl-final.png). It shows lifted shadow detail with directional shadows retained. The pine canopy is still dark and the technical scene remains sparse.

This is a practical approximation, not a complete split-sum IBL solution: diffuse irradiance uses second-order spherical harmonics, while specular uses box-filtered mip levels without GGX prefiltering or a BRDF LUT. The HDRI is not drawn as a visible sky. Those remain follow-up rendering work.
