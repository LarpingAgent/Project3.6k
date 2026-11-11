# Decision 0009: add ambient and directional diffuse lighting to the static forest

## Context

The current scene is a small planar low-poly composition. Its vertex colors and sampled texture provide albedo, but there is no lighting response. The existing triangles are not a spatial forest mesh from which useful 3D normals can be derived.

## Options considered

- Use the same camera-facing normal for every surface: minimal data, but all albedo receives the same diffuse factor and the light has no visible low-poly response.
- Author a few normals for the static semantic surfaces and implement ambient plus Lambert diffuse: gives canopy facets directional variation with no new renderer resources.
- Build volumetric trees and generate normals from their mesh: improves scene depth and natural shading, but broadens this renderer milestone into asset and geometry redesign.

## Decision

Add a normal attribute to the static vertex stream. Use authored, normalized world-space normals for the ground, ridge, trunks, and three canopy facets. In the fragment shader, multiply the sRGB-decoded sampled texture and vertex tint by a fixed world-space directional Lambert term with a constant 0.26 ambient floor. Keep light parameters as shader constants for this static scene; there is no lighting uniform or per-frame CPU work.

## Consequences

Canopy facets respond differently to the fixed light while the scene remains inexpensive and compatible with the Vulkan 1.0 pipeline. Because the scene is still planar and its normals are authored, this is a starter lighting model rather than a general material/lighting system. Mesh-derived normals, dynamic lights, specular response, shadows, and PBR remain future work. Interactive shading quality is recorded in the manual checklist.

## Follow-up

Decision [0013](0013-spatial-low-poly-forest.md) replaces the planar silhouettes with shallow prisms and a ground plane. The fixed lighting model remains; the geometry builder now supplies face and side normals. Interactive shading quality remains a manual check.
