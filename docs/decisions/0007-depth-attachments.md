# Decision 0007: allocate one depth attachment per swapchain image

## Context

The perspective camera needs a depth test, and multiple CPU frames may be in flight at once. A single shared depth image would be written by overlapping submissions unless additional serialization were added.

## Options considered

- One depth image shared by every framebuffer: lowest memory use, but would require serializing otherwise independent swapchain images.
- One depth image per frame in flight: possible, but a swapchain image can be used by a different frame slot after acquisition.
- One depth image per swapchain image: naturally paired with each color image and safe to reuse after that image's in-flight fence is waited.

## Decision

Choose the first supported optimal-tiling depth format from `D32_SFLOAT`, `D32_SFLOAT_S8_UINT`, `D24_UNORM_S8_UINT`, and `D16_UNORM`. Allocate a device-local depth image and view per swapchain image, pair it with that image's color view in the framebuffer, clear depth to 1.0, and use a `LESS` test with depth writes enabled. Recreate these resources with the swapchain extent and destroy framebuffers before their views and images.

The static demo uses separate indexed draw ranges for ground, trees, and ridge. The ridge is deliberately submitted after nearer trees; correct depth testing keeps it behind them regardless of submission order.

## Consequences

Depth resources scale with the swapchain image count and extent. The design avoids extra queue serialization and keeps reuse tied to the existing per-image fence path. A future renderer with many large targets can reevaluate memory aliasing or transient depth allocation based on measured memory pressure.
