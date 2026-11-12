# Manual test checklist

Items marked **NOT VERIFIED BY AUTOMATION** require an interactive check. Automated Vulkan checks are recorded in `target/validation-runs/` and are not a substitute for the checks below.

## Phase 4a CPU asset pipeline

- `cargo test -p project3-6k-assets` verifies synthetic glTF/GLB fixtures and imports the four curated Poly Haven glTF models from `assets/reference_scene/polyhaven/`, including their external JPEG PBR maps.
- `cargo test -p project3-6k-assets imports_curated_polyhaven_reference_models_from_disk -- --nocapture` prints the imported mesh, instance, material, image, and retained CPU-byte counts for those real files.
- **NOT VERIFIED BY AUTOMATION** — Open a separately supplied glTF/GLB with embedded and relative PNG/JPEG resources; confirm validation errors identify unsupported content and malformed external paths cleanly.
- **NOT VERIFIED BY AUTOMATION** — Confirm each added asset has a source, author, license, and attribution record in `assets/ASSET_SOURCES.md`.

## Phase 4b renderer image evidence

- `cargo run -p project3-6k-app -- --run-frames 5 --validation --capture-frame artifacts/visual-verification/baseline-before-rendering-upgrade.png` captures one real swapchain frame and exits only after five submitted frames. This baseline was captured and opened successfully; inspect it before comparing later builds.
- **NOT VERIFIED BY AUTOMATION** — After PBR/shadow changes, save a second PNG beside the baseline and compare surface maps, material response, shadow contrast, and foliage detail at the same window extent.

## Rendering and asset foundation

- `cargo test --workspace` exercises bounded glTF/GLB import, real Poly Haven model decoding, texture mip reduction, model cache limits, vertex layout, and the technical ground plane.
- `cargo clippy --workspace --all-targets -- -D warnings` checks all workspace targets without warnings.
- `cargo run -p project3-6k-app -- --run-frames 5 --validation --capture-frame artifacts/visual-verification/current.png` opens the application, imports and uploads the external models/maps, renders, captures one swapchain frame, validates, and exits.
- **VERIFIED** — [pbr-foundation-stage-3.png](../artifacts/visual-verification/pbr-foundation-stage-3.png) was captured from the running Vulkan application and opened at 1280×720. The floor, stump, rocks, upright trunk, foliage, PBR map response, and directional light are visible.
- **VERIFIED** — [hdr-stage-1.png](../artifacts/visual-verification/hdr-stage-1.png) was captured from the native Vulkan application after rendering to an FP16 target and running the separate ACES tone-mapping pass. `--validation` completed with exit code 0.
- **NOT VERIFIED / NOT IMPLEMENTED** — The HDR screenshot has no cast shadows or atmospheric fog; a shadow-map pass and further visual verification are still required.
- **NOT VERIFIED BY AUTOMATION** — Check the technical scene on a second Vulkan GPU, inspect foliage alpha edges and texture tiling at multiple distances, and compare a capture after FP16 HDR and shadow-map work.

## Phase 2 application and window lifecycle

- **NOT VERIFIED BY AUTOMATION** — Run `cargo run -p project3-6k-app`, confirm the window appears, press Escape, and confirm it closes cleanly.
- **NOT VERIFIED BY AUTOMATION** — Close the window with its title-bar close button and confirm the same shutdown path completes.
- **NOT VERIFIED BY AUTOMATION** — Resize the window repeatedly by dragging its edge and confirm it redraws at the new size without visible corruption.
- **NOT VERIFIED BY AUTOMATION** — Minimize and restore the window; confirm drawing suspends while minimized and resumes after restore.
- **NOT VERIFIED BY AUTOMATION** — Confirm the low-poly forest geometry appears over the forest-colored background and the window remains responsive during a long run.

## Phase 3c first-person camera

- **NOT VERIFIED BY AUTOMATION** — Run the app and left-click the window; confirm the cursor is hidden and mouse movement turns the view smoothly without the pointer reaching screen edges.
- **NOT VERIFIED BY AUTOMATION** — Press W/A/S/D and confirm movement follows the view's horizontal direction; press Space and either Shift key to move vertically. Confirm diagonal movement is not faster.
- **NOT VERIFIED BY AUTOMATION** — Press Escape once and confirm the cursor is released while the window remains open; press Escape again and confirm the app closes normally.
- **NOT VERIFIED BY AUTOMATION** — Focus another window while captured, then return; confirm held keys are cleared and the cursor is released safely.

## Phase 3d depth

- **NOT VERIFIED BY AUTOMATION** — Confirm the forest ridge remains behind the trees even though its indexed draw range is submitted after them; resize repeatedly and confirm the depth image recreates without flicker or stale occlusion.

## Phase 3e texture

- **NOT VERIFIED BY AUTOMATION** — Confirm the forest's repeating noise texture is visibly sampled and vertex colors tint it without unexpected color shifts, missing faces, or obvious seams.
- **NOT VERIFIED BY AUTOMATION** — Move and rotate the camera across near/far geometry and resize repeatedly; confirm texture filtering changes smoothly with distance and texture sampling remains stable after swapchain recreation.

## Phase 3f basic lighting

- **NOT VERIFIED BY AUTOMATION** — Confirm the two folded faces on each canopy show different brightness, trunks and ground remain visible under the ambient floor, and lighting stays fixed in world space while the camera turns. Cast shadows are not implemented.
- **NOT VERIFIED BY AUTOMATION** — Move around the scene and resize the window; confirm normals do not produce abrupt shading seams or black faces.

## Phase 4 profiling and debug display

- **NOT VERIFIED BY AUTOMATION** — Run for at least 120 frames and confirm the title bar refreshes with FPS, average/p95 frame interval, renderer-call CPU time, and current camera position.
- **NOT VERIFIED BY AUTOMATION** — Move the camera and confirm the position values update; compare the displayed FPS and timing values with a longer run to make sure the rolling window recovers after a brief hitch.

## Phase 5 spatial technical demo

- **NOT VERIFIED BY AUTOMATION** — Run the app and confirm the horizontal ground sits beneath the tree bases, the shallow prisms read as 3D shapes, and the distant ridge remains behind the trees while moving the camera.
- **NOT VERIFIED BY AUTOMATION** — Inspect ground, canopy, trunk, and side faces for visible texture detail, expected light-to-dark changes, missing faces, texture seams, and abrupt normal changes.

## Future manual checks

- Add checks for the final forest demo's visual quality and behavior on a second Vulkan-capable GPU when one is available.
