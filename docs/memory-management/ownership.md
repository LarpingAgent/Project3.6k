# Memory and resource ownership

## Current CPU ownership

- `FrameClock` stores timestamps and an integer frame counter. `tick` performs no heap allocation.
- `InputState` allocates only when the set of held keys changes; raw mouse deltas accumulate in a fixed two-value array.
- The platform-owned `Camera` stores small vector/scalar state and performs no per-frame allocation.
- Vulkan object collections, per-swapchain-image depth attachments, and static forest vertex/index buffers are created during initialization or swapchain recreation, not during normal frame submission.
- `ValidationState` uses atomics for counts and allocates message strings only when the Vulkan callback reports a warning or error.
- The platform application owns the `Window` and `VulkanRenderer`; its `Drop` implementation takes the renderer first so the surface dies before the native window.
- `--run-frames N` and `--auto-close` request exit through the normal event loop. The app's shutdown log is emitted only after the renderer (and its Vulkan RAII owners) and native window have been dropped.

## GPU ownership and destruction

`VulkanContext` owns the instance, surface, debug messenger, logical device, and an `Arc<ValidationState>`. The callback's user pointer refers to that allocation; it remains alive through `vkDestroyDebugUtilsMessengerEXT`. `SwapchainState` owns the swapchain loader, forest pipeline, device-local vertex/index buffers (including authored normals), and other swapchain-dependent objects. It allocates one depth image/view per swapchain image; that depth target is reused only after the image's previous submission fence is waited. `ForestScene` owns the sampled forest texture image, memory, view, linear repeat sampler and mip count, as well as one host-visible camera uniform buffer and descriptor set per frame in flight. Texture mip levels are decoded and generated on the CPU, packed into one temporary host-visible staging buffer, copied during initialization, then transitioned to shader-read-only; staging memory is released after the graphics queue is idle. Descriptor binding 1 references the scene texture for all frame slots. Directional-light and ambient values are static shader constants, so they require no extra GPU allocation or per-frame update. The current frame fence is waited before the matching camera buffer is mapped and updated; non-coherent memory is flushed before unmapping. `VulkanRenderer` declares the swapchain state before the context so Rust drops it first; `SwapchainState::drop` waits for device idle and destroys synchronization, command pool, framebuffers, scene pipeline, descriptor pool/layouts, uniforms, texture sampler/view/image/memory and mesh buffers, depth views/images, color image views, render pass, and swapchain in dependency-safe order.

Swapchain images are owned by the presentation system and are never freed directly. Image views, framebuffers, command pools, semaphores, fences, render pass, and swapchain are explicitly destroyed by their owner. Partial initialization is handled by initializing resource state to null handles and draining only successfully created objects.

## Unsafe boundaries

Unsafe code is limited to Vulkan FFI and native handle interop. Each call documents the handle validity, lifetime, and synchronization assumptions at the call site. The debug callback copies Vulkan's temporary message into owned text and does not retain callback data pointers. Its user pointer is the stable `Arc` allocation described above. No unsafe code is exposed through the public API.

## Deferred allocator decisions

The demo creates device-local vertex/index and texture allocations, plus temporary host-visible staging allocations. It prefers coherent host memory and flushes the whole mapped allocation when only non-coherent memory is available. These static allocations do not justify `gpu-allocator` or a custom suballocator yet; see [Decision 0005](../decisions/0005-static-mesh-memory.md). The swapchain waits for device idle before destroying scene resources, which is simple and safe for initialization/resize but can stall. Dynamic asset and world resources will need fence-based retirement before memory is reclaimed.
