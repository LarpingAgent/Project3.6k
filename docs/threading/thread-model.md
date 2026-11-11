# Thread model

The initial application is single-threaded on the CPU. `winit` dispatches native keyboard, raw mouse, and focus events; the platform layer updates input, camera pose, frame timing, and rolling profiling; Vulkan command recording/submission is performed from the same event-loop thread. The fixed-size profiler records redraw intervals and wall time inside the renderer call without allocating. That renderer-call value includes CPU waits and API calls; it does not measure GPU execution. The GPU and presentation engine execute submitted work asynchronously, ordered by explicit semaphores and fences. A frame's uniform buffer is written only after its fence signals.

The Vulkan validation callback writes through atomic counters and a mutex-protected message list. This keeps its monitor safe if a Vulkan implementation invokes the callback from a different thread; the monitor is read after renderer teardown.

This model keeps window and surface ownership simple while the renderer is being established. No job system or worker pool is added before asset loading or world streaming needs one.

Future asynchronous work should use owned messages or immutable snapshots across thread boundaries. Window events remain on the event-loop thread. Worker threads should not mutate renderer-owned Vulkan objects directly; uploads should be handed to a renderer-owned transfer path with explicit synchronization and resource lifetime tracking.
