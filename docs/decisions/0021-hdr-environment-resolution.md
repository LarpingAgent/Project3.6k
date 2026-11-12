# Decision 0021: Raise the reference HDR environment to 2K

## Context

The full-screen sky made the environment panorama visible at 1280×720. The original 1K source (1024×512) was upscaled vertically and looked soft in the captured scene.

## Decision

- Replace the 1K Forest Slope Radiance HDRI with Poly Haven's official 2K version (2048×1024), retaining its CC0 source and API-reported digest in the asset records.
- Keep the same bounded CPU importer, spherical-harmonic integration, FP16 upload, and scene-owned resource lifetime.
- Keep the full mip chain; do not increase model texture resolution in this change.

## Consequences

The visible backdrop is sharper at 1280×720. The mip payload increases from 5,592,408 bytes to 22,369,624 bytes on the GPU, while retained CPU mip data increases from 11,184,816 bytes to 44,739,248 bytes. Environment import/upload rose from about 35 ms to about 138 ms; measured startup to the rendered-ready scene was about 1.50 seconds.

On the GTX 1660 SUPER the 600-frame Release run remained near the 75 Hz display cap at 74.8–77.3 FPS, with 13.8–14.3 ms p95 intervals. Keyboard input occurred during the run, so treat these as interactive desktop measurements. The 2K screenshot is [forest-pine-skybox-2k-final.png](../../artifacts/visual-verification/forest-pine-skybox-2k-final.png).
