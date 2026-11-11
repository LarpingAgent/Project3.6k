# Manual test checklist

Items marked **NOT VERIFIED BY AUTOMATION** require an interactive check. Automated Vulkan checks are recorded in `target/validation-runs/` and are not a substitute for the checks below.

## Phase 2 application and window lifecycle

- **NOT VERIFIED BY AUTOMATION** — Run `cargo run -p project3-6k-app`, confirm the window appears, press Escape, and confirm it closes cleanly.
- **NOT VERIFIED BY AUTOMATION** — Close the window with its title-bar close button and confirm the same shutdown path completes.
- **NOT VERIFIED BY AUTOMATION** — Resize the window repeatedly by dragging its edge and confirm it redraws at the new size without visible corruption.
- **NOT VERIFIED BY AUTOMATION** — Minimize and restore the window; confirm drawing suspends while minimized and resumes after restore.
- **NOT VERIFIED BY AUTOMATION** — Confirm the low-poly forest geometry appears over the forest-colored background and the window remains responsive during a long run.

## Future renderer milestones

- Add checks for camera movement, mouse capture/release, depth ordering, texture appearance, and lighting when those features exist.
- Add checks for the final forest demo's visual quality and behavior on a second Vulkan-capable GPU when one is available.
