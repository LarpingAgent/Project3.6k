# Decision 0002: Use a Vulkan 1.0 clear-frame path first

- Status: accepted
- Date: 2026-11-11

## Context

The first rendering milestone needs to prove instance/device/surface/swapchain setup and synchronized presentation. The machine's driver reports Vulkan 1.4, but the environment has no Vulkan SDK or validation layer.

## Decision

Use `ash` directly and request Vulkan 1.0. Dynamically load the system loader. Use a conventional render pass and FIFO swapchain presentation, and make validation-layer use optional based on availability. Do not depend on dynamic rendering or shader tools for this stage.

## Consequences

The current executable can run without Vulkan SDK headers or link-time libraries. Shader compilation, validation diagnostics, geometry rendering, and feature-level selection remain follow-up work. The driver can be checked locally with `vulkaninfo --summary`.
