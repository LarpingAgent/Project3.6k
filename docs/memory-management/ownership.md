# Memory and resource ownership

## Current CPU ownership

- `FrameClock` stores timestamps and an integer frame counter. `tick` performs no heap allocation.
- `InputState` allocates only when the set of held keys changes.
- Vulkan object collections grow during initialization or swapchain recreation, not during normal frame submission.
- `ValidationState` uses atomics for counts and allocates message strings only when the Vulkan callback reports a warning or error.
- The platform application owns the `Window` and `VulkanRenderer`; its `Drop` implementation takes the renderer first so the surface dies before the native window.
- `--run-frames N` and `--auto-close` request exit through the normal event loop. The app's shutdown log is emitted only after the renderer (and its Vulkan RAII owners) and native window have been dropped.

## GPU ownership and destruction

`VulkanContext` owns the instance, surface, debug messenger, logical device, and an `Arc<ValidationState>`. The callback's user pointer refers to that allocation; it remains alive through `vkDestroyDebugUtilsMessengerEXT`. `SwapchainState` owns the swapchain loader and all swapchain-dependent objects. `VulkanRenderer` declares the swapchain state before the context so Rust drops it first; `SwapchainState::drop` waits for device idle and destroys synchronization, command pool, framebuffers, image views, render pass, and swapchain in dependency-safe order. Context destruction waits defensively, then destroys the device, debug messenger, surface, and instance in reverse creation order.

Swapchain images are owned by the presentation system and are never freed directly. Image views, framebuffers, command pools, semaphores, fences, render pass, and swapchain are explicitly destroyed by their owner. Partial initialization is handled by initializing resource state to null handles and draining only successfully created objects.

## Unsafe boundaries

Unsafe code is limited to Vulkan FFI and native handle interop. Each call documents the handle validity, lifetime, and synchronization assumptions at the call site. The debug callback copies Vulkan's temporary message into owned text and does not retain callback data pointers. Its user pointer is the stable `Arc` allocation described above. No unsafe code is exposed through the public API.

## Deferred allocator decisions

The first phase does not allocate user buffers or textures, so no general GPU allocator is introduced. Before adding asset uploads, measure allocation patterns and choose between a small explicit device-memory allocator and a maintained Vulkan allocator crate. Resource destruction must be deferred until GPU work that references a resource has completed; frame fences or a later timeline/deferred-release queue can provide that boundary.
