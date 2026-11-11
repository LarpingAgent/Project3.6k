# Architecture overview

Project3.6k starts with the smallest set of modules that perform real work. It is a Cargo workspace, not a collection of placeholder crates.

```mermaid
flowchart LR
  app[app] --> platform[platform]
  platform --> core[core]
  platform --> vulkan[vulkan]
  platform --> window[winit window]
  vulkan --> driver[Vulkan loader and driver]
```

## Ownership boundaries

The platform crate owns the native `Window` and `VulkanRenderer`. The Vulkan crate owns the instance, surface, device, swapchain, render pass, framebuffers, command buffers, and synchronization primitives. `FrameClock` owns no system resources and allocates nothing per tick.

The application explicitly drops the renderer before the native window because the Vulkan surface is tied to the window handles. The Vulkan renderer destroys swapchain resources before its logical device, surface, debug messenger, and instance.

## Data flow

Native key events update `InputState`; losing window focus clears held keys. The current foundation only uses Escape to close. Resize events mark the renderer suspended for zero-sized windows or request swapchain recreation for a valid size.

The renderer owns GPU synchronization and submission. The platform layer owns event dispatch and frame pacing. Neither layer depends on gameplay systems.

## Planned growth

When needed, asset, math, scene, world, physics, and audio crates can be introduced with explicit interfaces. A rendering pipeline and resource allocator should be added before any advanced effects. World streaming should use bounded budgets and asynchronous loading rather than retaining the complete world in memory.
