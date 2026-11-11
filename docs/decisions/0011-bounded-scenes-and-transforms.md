# Decision 0011: represent the starter scene with bounded mesh nodes and TRS transforms

## Context

The forest renderer currently stores one flat vertex array and hard-codes vertex positions at GPU upload time. Phase 4 calls for a platform-neutral scene representation and transform system, while the immediate demonstration remains static and uses one indexed mesh.

## Options considered

- Keep all positions and depths embedded in the renderer's vertex conversion: minimal code, but scene membership and world transforms are implicit.
- Add a small core `Scene` of mesh handles and value-owned TRS transforms, then bake static transforms into the initial GPU buffer: creates a real renderer-independent boundary with no dynamic frame cost.
- Build a full hierarchical ECS and per-object GPU draw system now: enables dynamic scenes, but adds identity, mutation, update, and synchronization policies without a current moving object.

## Decision

Add a bounded `Scene` to the core crate. A scene stores ordered nodes that reference opaque mesh handles and validated translation/rotation/scale transforms. Transform creation rejects non-finite and singular inputs, normalizes quaternions, and supplies point and inverse-transpose normal transforms. The renderer builds the five static forest nodes, applies their transforms while assembling the vertex upload, and uses node order to submit corresponding indexed ranges.

## Follow-up

Decision [0013](0013-spatial-low-poly-forest.md) expands this scene to six bounded nodes: the ground plane, forest panel, three trees, and distant ridge. The static geometry builder now generates the mesh vertices and normals before applying the same one-time transform bake and indexed-range mapping.

## Consequences

Core scene data has no dependency on Vulkan or platform input. Static geometry incurs no per-frame transform work. Runtime transform changes currently require rebuilding/updating GPU vertex data; dynamic object buffers, hierarchical parents, materials, and entity deletion policies remain future extensions. Scene capacity is explicit to avoid unbounded node growth.
