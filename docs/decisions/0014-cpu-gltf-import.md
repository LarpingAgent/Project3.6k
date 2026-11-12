# Decision 0014: bounded CPU glTF and raster asset import

## Context

The renderer has a bounded PNG cache but no external model path. PBR rendering requires positions, normals, tangents, UVs, material factors, and correctly identified texture maps. The workspace supports Rust 1.85, so recently raised dependency MSRVs must not silently move the project forward.

## Options considered

- Parse glTF JSON and binary chunks by hand: avoids dependencies, but duplicates a large file-format parser and validation surface.
- Use a full engine importer such as a scene framework: provides more runtime features than this Rust + `ash` renderer needs and risks coupling asset processing to GPU ownership.
- Use `gltf` for glTF 2.0 parsing/accessors and `image` for raster decoding, keeping CPU conversion in `project3-6k-assets`: focused formats, explicit geometry conversion, and no Vulkan dependency.

## Decision

Use `gltf` 1.4.1 with its `utils` and `names` features and `image` 0.25.9 with only common raster decoders enabled. `gltf` is MIT/Apache-2.0 and declares Rust 1.61. `image` 0.25.10 raised its MSRV to Rust 1.88, so pin the latest compatible 0.25.9 release, which declares Rust 1.85. The implementation does not enable the convenience importer; it bounds source/resource sizes, validates local external paths, decodes images under dimension/allocation limits, and constructs renderer-independent CPU structures itself.

The importer accepts static triangle, triangle-strip, and triangle-fan primitives; it expands them to triangle lists, preserves node transforms and PBR texture references, and generates missing normals/tangents. It explicitly rejects skinned/morphed data and line/point primitives. CPU images retain raw decoded channel values so Vulkan can select sRGB formats for color maps and linear formats for normal/metallic-roughness/AO maps.

`ModelAssetCache` uses count/byte limits and generational handles. `AsyncModelLoader` runs one CPU worker with bounded request and completion queues. GPU upload, GPU cache ownership, deferred GPU destruction, and streaming remain in the Vulkan phase.

## Consequences

The assets crate gains focused decode dependencies but remains independent of Vulkan. The pinned image release preserves the declared workspace MSRV. Initial import intentionally excludes skinning, morph targets, remote URIs, and glTF extensions that change texture coordinates; unsupported features need explicit handling before sample assets relying on them are accepted.

Asset author, source, and license information is recorded in [ASSET_SOURCES.md](../../assets/ASSET_SOURCES.md) before external files are added.
