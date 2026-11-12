# Current memory and resource ownership

This describes the implementation after Phase 2. Target ownership rules for ECS, a unified asset registry, and open-world streaming are in [Master Engine Architecture](../architecture/master-architecture.md).

## CPU ownership

- `EngineApplication` owns `Window`, `VulkanRenderer`, `EngineRuntime`, `Camera`, and `FrameProfiler`. It explicitly shuts down Engine, then takes/drops the renderer before the window because the Vulkan surface depends on the native window handles.
- `EngineRuntime` owns its validated `EngineConfig`, `FrameClock`, fixed-step accumulator/index, bounded `VecDeque<EngineEvent>` (default maximum 2,048; configured maximum 65,536), and a fixed 8,208-byte logical action/pointer state for all `u16` actions. The event queue cannot grow beyond its configured capacity; a full queue returns a typed error. No references or GPU resources cross the crate boundary.
- `FrameClock` stores monotonic timestamps and a frame counter. `FrameProfiler` remains in Platform and stores fixed-capacity samples; normal recording does not grow a vector.
- Platform maps `winit::KeyCode` to stable `ActionId` values. Engine stores logical action state and pointer delta without depending on `winit`.
- Core `Scene` owns a bounded `Vec<SceneNode>` with value-owned mesh IDs and transforms. It is a prototype and is not the data source for the current Vulkan forest scene.
- `AssetManager` owns decoded PNG texture mip chains under byte/count/key limits. `ModelAssetCache` owns imported CPU model payloads under byte/count budgets. Both use generation-checked handles. They are implemented and unit-tested but are not integrated into the current renderer's asset load path.
- `AsyncModelLoader` owns one worker, bounded synchronous request/completion channels, request IDs, and the worker join handle. The worker owns temporary file/decode buffers and returns an owned `CpuModelAsset` or typed error. Dropping the loader closes channels and joins the worker.
- Current app startup submits a fixed list of models and waits for all completions before upload. The environment importer and its GGX prefilter are synchronous. CPU cache residency is not persistent across swapchain recreation.

## Vulkan and GPU ownership

- `VulkanContext` owns the Vulkan entry/instance, surface, debug messenger/state, physical/logical device, and queue selection. Validation callback state is reference-counted and remains alive through debug-messenger destruction.
- `VulkanRenderer` owns `SwapchainState` and `VulkanContext`. `SwapchainState` owns swapchain images/views, per-image depth/HDR resources, per-frame shadow targets, render passes, `ForestScene`, tone-map pass, framebuffers, command resources, semaphores, fences, and in-flight image tracking.
- `ForestScene` owns the GPU model cache, model/ground/environment textures, material descriptors, scene uniform buffers, pipelines, and static placements. The GPU cache uses generation-checked handles and an estimated 256 MiB resident model-buffer/texture budget.
- `BufferAllocation`, `TextureResource`, `DepthAttachment`, `HdrTarget`, `ShadowTarget`, and tone-map resources own their Vulkan allocation and destroy it through `Drop`. The owning renderer waits before resources that may be in use are destroyed.
- CPU import and Vulkan upload are separate modules. Temporary staging buffers are freed after upload queue completion. Current texture/model upload paths use queue waits; final shutdown and swapchain recreation may wait for device idle.
- The renderer keeps persistent asset caches inside the swapchain-owned scene. A nonzero-size resize can therefore rebuild the scene and reimport/reupload assets; this is safe for the current test app but causes an avoidable hitch and must be separated before runtime streaming.

## Current budgets and measured residency

Default model import limits include 256 MiB per-model combined CPU import memory, 2,000,000 vertices, and 6,000,000 indices. The GPU model cache is capped at 32 models and 256 MiB of estimated resident buffers/texture mip payload. CPU PNG texture and model cache budgets are configurable but are not a single unified registry budget.

At the audit run, the renderer logged 138,377,796 bytes for model GPU resources, 16,777,212 bytes for ground GPU resources, 22,850,224 bytes for environment GPU mip/LUT payload, and 45,700,448 retained CPU environment bytes. Render targets, shadow images, driver allocations, staging peaks, and allocator overhead are outside those logged payload counts; they are not total process/GPU-memory figures.

## Unsafe and synchronization boundaries

Unsafe code is concentrated in Vulkan FFI/native-handle interop. Call-site comments record handle and synchronization assumptions. A frame's uniform is written after its in-flight fence is waited. Swapchain images are presentation-owned and not destroyed directly. Validation logs zero warnings/errors in the audit capture and `Drop` releases renderer before the window.

Engine state is only mutated by `EngineApplication` on the winit event thread. `EngineRuntime::shutdown` clears queued events and input state without allocating a replacement queue; the runtime owns no worker or shared mutable state.

## Required future ownership changes

Before phases 4–6, unify asset identity and unload semantics without merging CPU and GPU ownership. ECS components should store stable asset handles, never GPU pointers. The Vulkan backend should retire resources only after the last referencing fence/timeline value signals; ordinary streaming should not use `device_wait_idle`. Swapchain resize must not discard unrelated world/model resources. See the ownership diagram and rules in the Master Architecture.
