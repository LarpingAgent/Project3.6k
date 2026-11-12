# Decision 0017: Directional shadows and redundant resize handling

## Context

The verified PBR and FP16 HDR scene had directional sun lighting but no cast shadows. Runtime profiling also showed that the window system reported a temporary size different from the created swapchain extent, followed by duplicate reports of the actual extent. The renderer treated every nonzero resize notification as a request to rebuild the swapchain and scene, which repeated the synchronous import and GPU uploads of every reference asset and made startup appear hung.

## Decision

- Add a depth-only, single-map directional shadow pass before the HDR scene pass. Keep two 1024×1024 sampled depth targets, one per frame in flight.
- Render opaque geometry and apply the same alpha mask to foliage in the shadow pass. Sample the shadow depth with four comparison taps and a small normal/depth bias.
- Keep cascaded shadows deferred until a larger scene and camera coverage require them; the current map is intentionally a compact technical-scene solution.
- Only request swapchain recreation when the nonzero window extent differs from the active swapchain extent. Zero-sized windows remain suspended, and an actual size change still follows the existing recreate path.
- Record a pure extent comparison unit test so a duplicate same-size notification cannot trigger a reload unnoticed.

## Consequences

The 1280×720 validation capture shows cast shadows from the trunk, rocks, stump, and masked shrubs. Shadow resolution and the four-tap kernel reduce the work compared with the initial 2048 map and nine-tap kernel. The scene still exhibits shadow aliasing and lacks cascades and environment lighting.

On the current Windows desktop, one same-size resize sequence previously caused the entire scene to load twice. After the fix, the four models import and upload once. A real extent change can still rebuild scene assets and stall because GPU uploads remain synchronous; separating persistent scene resources from swapchain resources is the next lifecycle improvement.

The 600-frame dev-build run on an NVIDIA GeForce GTX 1660 SUPER at 1280×720 reported 74.9–78.1 FPS, 12.8–13.35 ms average frame interval, and 14.7–15.0 ms p95 in the rolling windows. Startup remained about 10.3 seconds, mostly due to synchronous first-load CPU import and GPU upload. See the current capture and full measurements in `docs/rendering/vulkan-pipeline.md`.
