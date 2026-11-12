# Decision 0013: build a spatial low-poly forest for the technical demo

## Context

The Phase 3 forest used camera-facing polygons with authored normals. That was enough to exercise texturing and lighting, but did not provide a ground surface or actual side faces for the Phase 5 technical demo.

## Options considered

- Keep the planar composition and defer spatial geometry: smallest code change, but the demo cannot exercise ground contact or side-face lighting.
- Replace the scene with a general model-loading pipeline: flexible, but adds importer, material, and asset-lifecycle scope beyond the current static demonstration.
- Generate a small bounded static mesh from the existing silhouettes: adds ground and prism faces while keeping the current scene, texture, lighting, and Vulkan upload path.

## Decision

Keep the six-node bounded scene and generate its upload geometry at initialization. Use a horizontal ground plane and extrude the forest panel, tree canopies/trunks, and distant ridge into shallow prisms. Fold each canopy front into two shallow geometric facets so its normals produce visible directional-light contrast; generate outward side and back normals, transform positions and normals through each node's TRS, and preserve one indexed range per node. Keep the existing static texture and ambient/directional shader lighting.

## Consequences

The technical demo now exercises spatial geometry, surface normals, texture sampling, perspective, and depth testing without adding a runtime importer or dynamic mesh updates. Geometry remains a small hard-coded demonstration; transforms are baked at initialization. Tests and Vulkan validation do not confirm the final appearance, tree-to-ground contact, or shading quality; those remain explicitly marked in the manual checklist.
