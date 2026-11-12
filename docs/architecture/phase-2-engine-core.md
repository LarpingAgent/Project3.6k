# Phase 2 — Engine Core Implementation Report

- **Status:** Implementation checks passed; focused closure review is in [Phase 2 Verification](phase-2-verification.md).
- **Scope:** lifecycle, event translation/dispatch, logical input, time/update scheduling, engine configuration/errors, and orderly host shutdown.
- **Architecture review:** [Phase 1 review](phase-1-architecture-review.md).

## Implemented

- Added `project3-6k-engine`, depending on `project3-6k-core`, `thiserror`, and `tracing`; it has no `winit`, Platform, Assets, or Vulkan dependency.
- Added checked `Created → Running → StopRequested → Stopped` lifecycle. Shutdown is idempotent and clears queued events/input without replacing or reallocating the bounded queue.
- Added owned platform-neutral events for exit, resize, focus, logical actions, and pointer motion. FIFO dispatch is bounded; overflow and non-finite pointer input return typed errors.
- Added logical `ActionId` input state, focus-loss clearing, and fixed-size storage for 65,536 action IDs. Platform maps physical `winit` keys to actions.
- Reused `core::FrameClock` and added fixed/variable scheduling with a 100 ms default frame-delta cap, a five-update default cap, interpolation alpha, tick indices, and explicit excess-time reporting. Caller limits are capped at 32 updates and 65,536 queued events.
- Kept the `winit` event pump, camera controller, profiler, `RunOptions`, `ApplicationError`, and Vulkan renderer in Platform. Engine updates the camera through the variable callback; Platform renders after Engine returns. No renderer, lighting, shadow, PBR, asset, ECS, terrain, or gameplay feature was added.
- Kept application tracing setup and added lifecycle/catch-up logging through `tracing`.

## Changed files

- Added `crates/project3-6k-engine/Cargo.toml`, `src/lib.rs`, `src/config.rs`, `src/event.rs`, `src/input.rs`, `src/runtime.rs`, and `tests/lifecycle_flow.rs`.
- Updated `Cargo.toml`, `Cargo.lock`, `crates/project3-6k-platform/Cargo.toml`, `crates/project3-6k-platform/src/application.rs`, `crates/project3-6k-platform/src/input.rs`, and `crates/project3-6k-platform/src/lib.rs`.
- Updated `docs/architecture/master-architecture.md`, `module-dependencies.md`, `overview.md`, added this report, and updated `docs/memory-management/ownership.md`, `docs/threading/thread-model.md`, and `docs/MANUAL_TEST_CHECKLIST.md`.

## Acceptance evidence

| Check | Result |
|---|---|
| Phase 1 architecture review | Passed with the Platform-host / Engine-core boundary correction documented before code changes. |
| Cargo dependency graph | `cargo metadata --no-deps` plus workspace-member graph traversal confirmed `app → platform → engine → core`, `platform → vulkan → assets/core`, with no cycles. Engine has no Platform/Vulkan dependency. |
| Lifecycle/event/update/render/shutdown | `cargo test -p project3-6k-engine --test lifecycle_flow`: passed. The test asserts start → FIFO event dispatch → fixed updates → variable update → host render → shutdown. |
| Deterministic behavior | Unit tests cover lifecycle transitions, bounded queue, input/focus clearing, frame-delta clamp, fixed-step cap, interpolation, event order, and invalid config. Platform test covers both Shift keys held and released separately. |
| Full workspace tests | `cargo test --workspace`: **61 passed, 0 failed** in the focused closure review. |
| Lint and build | `cargo clippy --workspace --all-targets -- -D warnings` and `cargo build --release -p project3-6k-app`: passed. `cargo fmt --all -- --check`: passed. |
| Native runtime | Release `--run-frames 180 --validation`: 180 frames submitted, clean shutdown, 0 Vulkan validation warnings/errors. Three additional Release `--auto-close --validation` runs also started/stopped cleanly with 0 warnings/errors each. |
| Resize/minimize/restore | Release `--run-frames 180 --resize-stress --validation`: exited 0; 8 resize events including one zero extent; restored and completed all 180 frames; 0 Vulkan validation warnings/errors. |

## Performance and memory evidence

- On the NVIDIA GeForce GTX 1660 SUPER at the current 1280×720 desktop path, a Release 180-frame validation run reported 77.19 FPS, 12.95 ms average frame interval, 13.77 ms p95, 26.73 ms maximum, and 13.00 ms average CPU time inside `VulkanRenderer::draw`. This is a display-paced application measurement, not an uncapped GPU benchmark or an isolated Engine-cost measurement.
- The engine action/pointer state is 8,208 bytes by construction. The default event queue is limited to 2,048 entries; caller configuration is validated and limited to 65,536 entries. Fixed update work is five ticks per frame by default and cannot be configured above 32.
- Three repeated startup/shutdown runs plus the resize stress run ended with clean Vulkan validation and orderly Rust/Vulkan teardown. No process-wide heap-leak detector was run, so this evidence is limited to lifecycle/validation behavior.

## Known issue and next phase

The resize stress run exposed the existing renderer behavior documented in Phase 0: each nonzero swapchain resize reconstructs the static forest scene and reimports/reuploads its assets. That caused multi-second hitches and fixed-step catch-up warnings of about 16.67 ms during the stress run. This remains outside Phase 2 and belongs to the rendering/resource lifecycle phases; the Engine correctly caps per-frame work and reports discarded simulation time.

No new screenshot was captured because Phase 2 made no rendering or visual-scene changes. The existing [Phase 0 baseline capture](../../artifacts/visual-verification/phase0-baseline.png) remains the renderer reference.

The Engine Core implementation is complete. Phase 2 closure awaits explicit user confirmation after the focused [verification review](phase-2-verification.md). Do not start Phase 3 before that confirmation.
