# Thread model

The initial application is single-threaded on the CPU. `winit` dispatches native events, the platform layer updates input and frame timing, and Vulkan command recording/submission is performed from the same event-loop thread. The GPU and presentation engine execute submitted work asynchronously, ordered by explicit semaphores and fences.

The Vulkan validation callback writes through atomic counters and a mutex-protected message list. This keeps its monitor safe if a Vulkan implementation invokes the callback from a different thread; the monitor is read after renderer teardown.

This model keeps window and surface ownership simple while the renderer is being established. No job system or worker pool is added before asset loading or world streaming needs one.

Future asynchronous work should use owned messages or immutable snapshots across thread boundaries. Window events remain on the event-loop thread. Worker threads should not mutate renderer-owned Vulkan objects directly; uploads should be handed to a renderer-owned transfer path with explicit synchronization and resource lifetime tracking.
