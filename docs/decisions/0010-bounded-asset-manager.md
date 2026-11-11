# Decision 0010: add a bounded CPU texture asset manager

## Context

The renderer needs PNG decoding and mip generation, while later scene and world systems will need assets without taking a dependency on Vulkan. The immediate engine phase needs a real loading/cache path, but not an asynchronous streaming system yet.

## Options considered

- Keep decoding inside the Vulkan renderer: least code, but asset formats and ownership become renderer-specific.
- Add an assets crate with keyed generational handles and explicit CPU memory/count limits: makes loading reusable and testable without creating GPU objects.
- Add a threaded streaming pipeline and GPU residency manager: supports larger worlds, but requires cancellation, upload scheduling, and deferred GPU retirement beyond the current static demo.

## Decision

Create `project3-6k-assets` as a renderer-independent workspace crate. It loads PNGs into RGBA8 CPU mip chains, caches repeated keys, exposes typed generation-tagged handles, and supports explicit unload. Configure decoded-byte, resident-byte, key-length, and asset-count limits. Vulkan consumes the loaded mip data to create its device-local image; the small static scene drops the CPU cache once initialization has completed.

Keep loading synchronous during startup. Add worker threads, cancellation, streaming policy, and GPU resource retirement only when a concrete dynamic-world requirement exists.

## Consequences

Asset parsing and CPU memory policy can be tested separately from Vulkan. The current renderer still loads one embedded PNG synchronously, so this crate establishes an independent interface without claiming runtime hot reload or streaming support.
