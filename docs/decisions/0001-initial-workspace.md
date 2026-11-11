# Decision 0001: Start with four concrete crates

- Status: accepted
- Date: 2026-11-11

## Context

The engine target contains many future subsystems, but an empty repository gives none of them a working interface yet. Empty placeholder crates would add dependency edges without shipping behavior.

## Decision

Create core timing, native platform/input, direct Vulkan presentation, and an app executable. Keep asset, scene, world, physics, audio, and gameplay out until their first real implementation is ready.

## Consequences

The app can demonstrate window lifecycle, input events, device selection, and safe frame submission. Future crates can be added with dependency direction documented before integration.
