# Decision 0006: use glam for camera math

## Context

Phase 3c needs tested vector movement and view/projection matrices shared by the platform camera and rendering boundary. Hand-written trigonometry and matrix assembly would duplicate common math and make Vulkan convention mistakes harder to catch.

## Options considered

- Hand-written vectors and matrices: no dependency, but more code to review and test for a core engine operation.
- `glam`: focused game/graphics math types with explicit right-handed camera helpers and Vulkan projection conventions.
- A general-purpose linear algebra crate: broad functionality beyond this engine's current camera need.

## Decision

Use `glam` 0.34.3 in `project3-6k-core`, pinned to that compatible minor line by the workspace manifest. The camera uses right-handed Y-up view space and `glam::camera::rh::proj::vulkan::perspective`, which produces Vulkan's [0, 1] depth and Y-down NDC convention. The Vulkan crate receives plain column-major `[[f32; 4]; 4]` data and does not depend on `glam`.

## Consequences

Core gains a direct math dependency, while platform and Vulkan remain separated. Camera projection, movement normalization, finite-input handling, pitch limits, and delta-time clamping can be unit-tested without a GPU. Keep the matrix at the crate boundary as a plain array so renderer APIs remain independent of a math library.
