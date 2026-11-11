# Decision 0003: validation and lifecycle gates

## Context

Vulkan resource-lifetime mistakes can be difficult to reproduce from a short interactive session. The foundation needs repeatable runs that exercise initialization, frame submission, resize, minimized-window suspension, and shutdown while making validation output a hard failure.

## Decision

- Debug builds always require `VK_LAYER_KHRONOS_validation` and `VK_EXT_debug_utils`; `--validation` can require them in release builds.
- The debug callback counts and logs warning/error messages. A run returns a nonzero exit code after normal resource teardown if either count is nonzero.
- `--run-frames N` counts successful queue submissions and exits through the normal event-loop shutdown path.
- `--resize-stress` requests multiple window sizes, minimizes and restores the window, and fails unless the event loop reports a zero extent.
- `scripts/validation-smoke.ps1` runs 1, 60, and 600 submitted frames and the resize stress sequence, checking exit codes, validation logs, and the zero-extent event.
- Phase 2's clear-only smoke gate did not include image readback. The Phase 3 renderer now has visible geometry, so offscreen/readback assertions belong in the renderer verification path before the foundation is considered complete.

## Consequences

Debug runs require the Vulkan SDK validation layer. Release runs can omit it unless `--validation` is requested. The smoke script is specific to the configured Windows/MSVC/Vulkan SDK environment. Interactive resize behavior still needs the manual checks listed in `docs/MANUAL_TEST_CHECKLIST.md`.
