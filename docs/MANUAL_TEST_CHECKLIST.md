# Manual test checklist

Items marked **NOT VERIFIED BY AUTOMATION** require an interactive check. Automated Vulkan checks are recorded in `target/validation-runs/` and are not a substitute for the checks below.

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

## Future renderer milestones

- Add checks for basic lighting and shading when that feature exists.
- Add checks for the final forest demo's visual quality and behavior on a second Vulkan-capable GPU when one is available.
