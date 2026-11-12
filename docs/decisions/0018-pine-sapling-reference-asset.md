# Decision 0018: Add a licensed pine group to the reference scene

## Context

The verified renderer scene had a flat ground plane, rocks, a stump, one dead trunk, and low shrubs. It exercised PBR and shadows but did not read as a forest because there was no living tree canopy.

## Decision

- Add Poly Haven's [Pine Sapling Small](https://polyhaven.com/a/pine_sapling_small), modeling by Rico Cilliers and photography by Rob Tuytel, licensed CC0.
- Keep the source 1K glTF/GLB-compatible model and relative JPEG PBR maps beside the model. Verify every downloaded file against the official API-reported MD5 and byte size and record it in `assets/reference_scene/polyhaven/DOWNLOADS.md`.
- Use the three tree variants already instanced by the glTF as one grouped placement. Do not generate replacement tree geometry.
- Limit this increment to one group placement at scale 3 while measuring the existing Release renderer. The source contains 398,144 triangles across three mesh instances; LOD, instancing, and frustum culling remain later work.

## Consequences

At this stage the Release reference scene imported five models and rendered seven placements. The model GPU cache reported 138,377,796 resident bytes, about 64.3 MB above the prior four-model scene. The 1280×720 validation screenshot is recorded at `artifacts/visual-verification/forest-pine-stage-2.png`. The canopy appeared dark because environment lighting had not yet been added; see Decision 0019 for the later HDR environment work.

The 600-frame Release run with one pine group reported 73.1–78.1 FPS, 12.8–13.7 ms average frame intervals, and 15.2–16.5 ms p95 on the NVIDIA GeForce GTX 1660 SUPER. A three-group trial looked denser but raised p95 to about 34–36 ms, so the current layout keeps one group. The pine canopy remains dark in the screenshot because this renderer has no environment image-based lighting and still uses a sparse technical layout; a complete forest scene is not claimed.
