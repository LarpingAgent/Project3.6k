# Decision 0012: report rolling CPU frame timing in the window title

## Context

The engine already uses monotonic timestamps for movement but only logs the latest frame delta. Phase 4/5 needs useful profiling and visible debug information without adding a debug UI dependency or allocating in the frame loop.

## Options considered

- Log every frame: simple but noisy and can distort timing through formatting and I/O.
- Add a debug GUI and Vulkan overlay: more flexible, but introduces another renderer path and resource lifetime before the demonstration needs one.
- Keep a fixed-size rolling sample array and publish summary metrics periodically in logs and the window title: visible, useful for the current app, and bounded.

## Decision

The core `FrameProfiler` retains the latest 120 redraw intervals and wall times spent inside `VulkanRenderer::draw` in a fixed array. Every 120 samples it reports estimated FPS, mean/p95/maximum frame interval, and mean renderer-call CPU duration. The platform layer writes these values plus camera position to the native window title and emits structured `tracing::debug` fields.

## Consequences

No heap allocation or GPU work is added to normal profiling samples. Title formatting and sorting happen once per reporting interval. The renderer-call value is CPU wall time, not a GPU timestamp; GPU query pools can be added when frame-level GPU profiling is needed. A configurable debug overlay remains deferred.
