# Current and target thread model

This file records the threads and scheduling that exist after Phase 2 and the constraints for future phases. The target schedule is defined in [Master Engine Architecture](../architecture/master-architecture.md).

## Current implementation

```mermaid
flowchart LR
  OS[OS / winit events] --> Main[Platform event + render thread]
  Main -->|owned platform-neutral events| Engine[EngineRuntime on same thread]
  Engine -->|fixed + variable update callbacks| Main
  Main -->|bounded request channel| Worker[one CPU glTF worker]
  Worker -->|owned completion channel| Main
  Main -->|Vulkan record + submit| GPU[GPU / presentation execution]
  GPU -->|fences + semaphores| Main
  Driver[validation callback thread] --> Monitor[atomics + mutex message list]
```

- `winit` invokes `EngineApplication` on one event-loop thread. Window handling, native input mapping, `EngineRuntime`, camera update, profiling, command recording, and submission all run on that thread.
- `about_to_wait` requests redraw and sets a wait-until frame deadline. Every redraw passes through the bounded Engine event queue, then the fixed-update accumulator and one variable update. The camera controller currently runs in the variable callback; no simulation systems consume fixed ticks yet.
- `EngineRuntime` is single-thread-owned (its `Rc` phantom marker makes it neither `Send` nor `Sync`). Its default event queue is capped at 2,048 events, configurable up to 65,536, and logical action state uses a fixed-size bitset; there are no event worker threads or cross-thread engine mutations.
- `AsyncModelLoader` starts one named CPU worker with bounded request and completion queues. It performs file read/import/validation only. Current startup waits for every requested completion, then the main thread uploads the assets.
- Environment HDR decode, GGX filtering, BRDF LUT generation, texture upload, and render-scene creation run synchronously during renderer initialization.
- GPU execution and presentation are asynchronous relative to CPU submission and are ordered with frame fences/semaphores. The app waits for the matching fence before updating frame-owned uniform data.
- Vulkan validation callbacks can arrive from an implementation-selected thread. They update atomic counts and retain owned warning/error messages behind a mutex; the application reads the report after renderer teardown.

## Target constraints

- Keep window and input mutation on the main/event thread.
- Keep World writes and initial Vulkan object creation on a single owner thread until profiling justifies another schedule.
- Cross-thread messages carry owned request IDs, generations, CPU payloads, or typed errors; no worker holds a borrow into World or renderer state.
- Bound both request and completion queues so uncollected assets apply back-pressure instead of growing memory without limit.
- Stale/cancelled requests are rejected by generation/request ID before they become resident.
- A future upload worker may prepare CPU staging data, but Vulkan object creation/submission stays on the backend owner unless explicit device/queue synchronization is designed.
- Do not add a generic job system in Phase 2. Phase 4 can expand the bounded loader; Phase 6 can add region jobs when chunk ownership/cancellation is specified.

## Current shutdown

The loader is dropped after startup completions are drained and joins its worker. `EngineApplication::drop` requests/stops Engine, releases cursor capture, takes/drops the renderer, then drops the window. The renderer waits for device idle before destroying swapchain-owned resources. The validation report is checked after application/renderer shutdown in validation mode.

The fixed scheduling API is implemented, but no simulation system uses it yet. These rules do not provide cancellation, streaming priorities, or fence-based per-asset retirement.
