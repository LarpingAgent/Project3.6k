# Decision 0005: memory strategy for the static demo mesh

## Context

The first mesh is a small static forest composition with one vertex buffer and one index buffer. It is created during renderer initialization and replaced only when swapchain resources are rebuilt.

## Options considered

- Write the final mesh directly to host-visible buffers: simplest, but may place frequently read vertex/index data in memory that is slower for the GPU.
- Upload through a host-visible staging buffer to device-local buffers: adds one initialization-time copy and explicit synchronization, then keeps frame reads on device-local memory.
- Add `gpu-allocator` or implement a custom suballocator: reduces allocation overhead once resource counts grow, but adds dependency or allocator complexity for only two persistent mesh buffers.

## Decision

Use one temporary host-visible staging buffer for the combined mesh, then copy into device-local vertex and index buffers. Prefer `HOST_COHERENT` staging memory and flush the whole mapped allocation on the non-coherent fallback. Record a transfer-write to vertex-input-read buffer barrier, submit the copy on the graphics queue, and wait for that queue before releasing staging memory.

Keep one Vulkan memory allocation per device-local buffer for this small demo. Do not add `gpu-allocator` or a custom suballocator until asset/scene counts and allocation lifetimes justify it. The swapchain path waits for device idle before releasing the scene buffers, so no resource is destroyed while a submitted frame can use it.

## Consequences

Initialization and resize perform a small synchronous upload. That is acceptable for this static mesh, but streamed assets will need asynchronous transfer and fence-based deferred retirement to avoid frame stalls. Because the demo scene currently lives with swapchain-dependent pipeline state, resize recreates and reuploads the mesh; separate scene lifetime from swapchain lifetime when the renderer becomes dynamic.
