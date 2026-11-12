# Decision 0020: Draw the HDR environment as the reference scene background

## Context

After environment lighting was added, the HDRI affected material shading but the scene still cleared to a solid dark-green color. The reference camera therefore had no visible forest backdrop or environmental horizon.

## Decision

- Add a dedicated full-screen triangle pipeline in the existing FP16 HDR render pass.
- Reconstruct a world-space view direction from the inverse camera view-projection matrix and sample the same equirectangular HDR environment used by PBR.
- Draw this background before scene geometry with depth testing and depth writes disabled. Keep the existing shadow and material pipelines unchanged.
- Keep the HDRI as a lighting/background reference; it is not geometry, terrain, or a gameplay world.

## Consequences

The reference scene now shows an external forest environment behind the technical ground and props. A 1280×720 Release capture is recorded at [forest-pine-skybox-final.png](../../artifacts/visual-verification/forest-pine-skybox-final.png). The backdrop is visibly soft at the retained 1K resolution and meets the flat technical ground at a hard horizon; a higher resolution environment and ground blending remain future visual work.

On the NVIDIA GeForce GTX 1660 SUPER, a 600-frame Release run with the sky pass measured 73.2–77.4 FPS, 12.9–13.7 ms average frame intervals, and 13.8–14.3 ms p95. The run received keyboard input and is therefore an interactive desktop measurement, not an isolated GPU benchmark. The sky uses one full-screen draw and does not add model geometry.
