# Decision 0004: compile GLSL with Vulkan SDK glslc

## Context

The renderer needs a reproducible path from human-readable GLSL to Vulkan 1.0 SPIR-V. The target environment already provides the Vulkan SDK and its `glslc` executable.

## Options considered

- `shaderc` as a Cargo build dependency: self-contained in Cargo, but adds a crate and native shader compiler build/link cost.
- The SDK's `glslc`: available in the required Windows development environment and compiles GLSL to SPIR-V without adding a Cargo dependency.
- Check in generated `.spv`: avoids a compiler at build time but makes shader source and binaries easy to desynchronize and commits build output.

## Decision

Use `build.rs` to invoke `$VULKAN_SDK/Bin/glslc.exe` with `--target-env=vulkan1.0`. Write binaries to Cargo's `OUT_DIR`, embed those outputs with `include_bytes!`, and ignore `.spv` files in Git. The GLSL sources remain the source of truth.

## Consequences

Building the Vulkan crate requires the configured Vulkan SDK. The runtime does not need shader compiler APIs and no third-party compiler dependency is added. If cross-platform toolchain support expands, the build script can select a platform-specific compiler path or a maintained shaderc build dependency can be reevaluated.
