# Vulkan rendering and asset pipeline

## Current initialization

1. Dynamically load Vulkan through `ash`, create the native surface, and select a graphics/present queue.
2. Create the swapchain, per-image depth and FP16 HDR targets, and two frames in flight.
3. Start one bounded CPU glTF worker and import five licensed static reference models, including three internally instanced pine sapling variants. CPU parsing and image decoding stay in `project3-6k-assets`; Vulkan owns GPU upload and resource lifetime.
4. Decode the licensed 1K Radiance HDR environment on the CPU, generate bounded linear RGBA mip levels, and integrate diffuse irradiance into nine real spherical-harmonic coefficients. Upload the environment as an FP16 sampled texture owned by `ForestScene`.
5. Upload model buffers and PBR maps through temporary staging allocations. Color/emissive textures use sRGB formats; normal, metallic-roughness, and AO maps use linear formats and role-correct mip reduction.
6. Upload the licensed forest-ground color, OpenGL normal, and ARM maps onto the tiled technical ground plane. Create material descriptors and per-frame scene uniforms, including compact diffuse irradiance coefficients.
7. Render a 1024×1024 directional shadow depth map for the current frame. In the HDR scene pass, draw the visible environment from a full-screen triangle and the camera inverse view-projection, then render GGX PBR geometry while sampling the shadow and HDR environment maps.
8. Sample the scene HDR image in a fullscreen pass, apply fitted ACES tone mapping, and present to the swapchain.

The CPU importer accepts bounded local glTF 2.0 and GLB static triangle data, retains node transforms and PBR maps, and rejects unsupported deformation, line/point primitives, and external paths that escape the model directory. `AsyncModelLoader` bounds the CPU work queue, but the current startup waits for all five imports and uploads synchronously on the renderer thread. Runtime asset streaming is not implemented.

The Vulkan GPU cache allows up to 32 models and 256 MiB of estimated resident model buffers and texture mip data. Generational handles reject stale references. Upload staging allocations are temporary. Cache entries and RAII resources are released after device idle; callers of `unload_after_device_idle` must honor that lifetime precondition. A redundant same-size window event is ignored to avoid unnecessary scene reloads. A real extent change still recreates the current scene and can repeat its synchronous asset load; persistent GPU assets across resize remain future work.

Imported materials currently support `TEXCOORD_0`, opaque and alpha-mask modes, base color, metallic, roughness, normal scale, AO strength, emissive, and double-sided normal handling. Alpha blend and alternate texture coordinate sets return typed errors instead of silently rendering incorrectly. The PBR fragment shader evaluates a GGX sun response, nine-coefficient diffuse environment irradiance, roughness-selected environment mip reflections, four-tap comparison-filtered shadows, normal mapping, and distance fog. Alpha-masked materials get a restrained opposite-side diffuse contribution to approximate foliage transmission. It writes linear radiance to FP16 HDR; a separate pass applies ACES tone mapping before presentation.

The environment importer is separate from Vulkan and enforces configured image dimensions, external-file size, and total CPU-memory limits. CPU data is converted to FP16 only in the Vulkan upload layer. `ForestScene` owns the environment texture and its lifetime; the general model cache remains independently budgeted. Model decoding already runs on the bounded CPU worker, while this single reference HDRI is imported and uploaded synchronously during startup. That adds about 35 ms on the measured machine. The visible sky pass reconstructs a world-space direction from the camera inverse view-projection and samples this same environment without adding sky geometry. General-purpose environment streaming and asynchronous GPU uploads remain unimplemented.

Diffuse irradiance uses a second-order spherical-harmonic approximation integrated over the equirectangular HDRI. Specular reflections sample ordinary box-filtered mip levels by roughness; they are not GGX-prefiltered and do not use a BRDF integration LUT. The HDRI supplies lighting only: there is no environment background or sky draw yet.

## Rendering dependency graph

```mermaid
flowchart LR
  CPU[glTF decode + validation] -->|CpuModelAsset| GPU[bounded Vulkan asset cache]
  HDRI[bounded HDR decode + mip generation + SH integration] --> EnvGPU[FP16 environment texture]
  EnvGPU --> Sky[full-screen HDR sky pass]
  Sky --> HDR[per-image FP16 HDR target]
  GPU --> Shadow[directional shadow depth pass]
  Shadow --> Scene[PBR reference scene]
  Scene --> HDR
  EnvGPU --> Scene
  GPU --> Scene
  HDR --> ToneMap[fullscreen ACES tone mapping]
  ToneMap --> LDR[LDR swapchain output]
  LDR --> Capture[optional PNG readback]
```

Implemented: external glTF/GLB and raster import; bounded Radiance HDR import, FP16 environment upload and mipmaps; CPU diffuse irradiance SH; visible HDR sky background; role-correct PBR texture formats and mipmaps; device-local mesh uploads; bounded GPU model cache and RAII lifetime; directional sunlight; GGX PBR; alpha-masked foliage with approximate back-side diffuse; 1024² single-map directional shadows with four comparison taps; distance fog; FP16 HDR; ACES tone mapping; and opt-in PNG swapchain readback.

Still open: GGX-prefiltered specular environment lighting and BRDF integration LUT, cascaded shadows, full terrain/forest generation, LOD and frustum culling, instanced vegetation, material blending, alternate UV sets, asynchronous GPU uploads, and world/asset streaming. The current shadow map covers one scene-sized area and is not a cascaded production solution. The reference layout is sparse and does not yet represent a finished forest.

## Visual verification and performance

The baseline is [baseline-before-rendering-upgrade.png](../../artifacts/visual-verification/baseline-before-rendering-upgrade.png). The PBR stage is [pbr-foundation-stage-3.png](../../artifacts/visual-verification/pbr-foundation-stage-3.png), HDR/tone mapping is [hdr-stage-1.png](../../artifacts/visual-verification/hdr-stage-1.png), shadow stage is [release-shadow-reference.png](../../artifacts/visual-verification/release-shadow-reference.png), previous pine scene is [forest-pine-stage-2.png](../../artifacts/visual-verification/forest-pine-stage-2.png), IBL-only scene is [forest-pine-ibl-final.png](../../artifacts/visual-verification/forest-pine-ibl-final.png), and the current visible HDR sky is [forest-pine-skybox-final.png](../../artifacts/visual-verification/forest-pine-skybox-final.png). The 1280×720 Release captures were opened and inspected: the HDRI now supplies a visible forest backdrop and environment lighting while directional shadows remain visible. The backdrop is soft at 1K resolution, the canopy remains dark, and the ground is a flat plane; this remains a renderer foundation rather than a finished forest.

Measured on an NVIDIA GeForce GTX 1660 SUPER at 1280×720 using the Release build with the visible sky pass, without capture or validation, over 600 submitted frames: rolling batches reported 73.2–77.4 FPS, 12.9–13.7 ms average frame interval, p95 13.8–14.3 ms, and a first-batch maximum of 26.9 ms (later maxima 14.1–26.7 ms). Occasional keyboard input reached the window during this run, so the p95 reflects an interactive desktop run rather than an isolated GPU benchmark. App startup to the rendered-ready scene was about 1.36 seconds: CPU model import about 0.22 seconds, model GPU uploads about 0.66 seconds, environment decode/upload about 35 ms, then scene/swapchain setup about 0.17 seconds. Resident model buffers and texture mips were 138,377,796 bytes; ground GPU resources were 16,777,212 bytes; FP16 environment mips were 5,592,408 bytes. These are one-machine measurements, not cross-GPU guarantees. Screenshot capture deliberately stalls a frame and must not be used for performance timing.

## Local verification

Set `VULKAN_SDK` to an SDK that provides `glslc`, then run from the repository root:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p project3-6k-app -- --run-frames 5 --validation --capture-frame artifacts/visual-verification/current.png
```

The Vulkan run opens the native application, loads external assets, renders, captures a real swapchain frame, checks validation, and exits. Compilation alone does not verify pixels. To inspect runtime profiling, use `$env:RUST_LOG='debug'; cargo run -p project3-6k-app -- --run-frames 600` and read the rolling profiler lines or window title after startup.

The current Release test executable is `target/release/project3-6k.exe`. Run it from this checkout so its external reference assets remain available. For interactive testing in PowerShell:

```powershell
.\target\release\project3-6k.exe
```

Click the window to capture the mouse, move with WASD, use Space/Shift to rise or descend, press Escape once to release the mouse, and press Escape again to close. This build loads the technical scene; it does not contain gameplay or a survival loop.

See [Decision 0014](../decisions/0014-cpu-gltf-import.md) for the CPU import library choice, [Decision 0015](../decisions/0015-vulkan-pbr-foundation.md) for the GPU cache/material foundation, [Decision 0016](../decisions/0016-fp16-hdr-tone-mapping.md) for HDR and tone mapping, [Decision 0017](../decisions/0017-directional-shadow-and-resize-startup.md) for the shadow pass and startup resize fix, [Decision 0018](../decisions/0018-pine-sapling-reference-asset.md) for the licensed pine reference asset, [Decision 0019](../decisions/0019-hdr-environment-ibl.md) for environment lighting, and [Decision 0020](../decisions/0020-hdr-sky-background.md) for visible environment rendering.
