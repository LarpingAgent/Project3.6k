use super::gpu_asset_cache::GpuVertex;

const GROUND_HALF_EXTENT: f32 = 16.0;
const GROUND_TILES: f32 = 8.0;

#[derive(Debug)]
pub(super) struct ForestGeometry {
    pub(super) vertices: Vec<GpuVertex>,
    pub(super) indices: Vec<u32>,
}

/// Creates the flat, texture-mapped technical test ground. No terrain is generated here.
pub(super) fn build_forest_geometry() -> ForestGeometry {
    let extent = GROUND_HALF_EXTENT;
    ForestGeometry {
        vertices: vec![
            GpuVertex {
                position: [-extent, 0.0, -extent],
                normal: [0.0, 1.0, 0.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
                color: [1.0; 4],
            },
            GpuVertex {
                position: [extent, 0.0, -extent],
                normal: [0.0, 1.0, 0.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                uv: [GROUND_TILES, 0.0],
                color: [1.0; 4],
            },
            GpuVertex {
                position: [extent, 0.0, extent],
                normal: [0.0, 1.0, 0.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                uv: [GROUND_TILES, GROUND_TILES],
                color: [1.0; 4],
            },
            GpuVertex {
                position: [-extent, 0.0, extent],
                normal: [0.0, 1.0, 0.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                uv: [0.0, GROUND_TILES],
                color: [1.0; 4],
            },
        ],
        // Counter-clockwise winding viewed from above (+Y).
        indices: vec![0, 2, 1, 0, 3, 2],
    }
}

#[cfg(test)]
mod tests {
    use super::{GROUND_HALF_EXTENT, build_forest_geometry};

    #[test]
    fn technical_ground_is_a_bounded_upward_facing_plane() {
        let ground = build_forest_geometry();
        assert_eq!(ground.vertices.len(), 4);
        assert_eq!(ground.indices, [0, 2, 1, 0, 3, 2]);
        assert!(
            ground
                .vertices
                .iter()
                .all(|vertex| vertex.normal == [0.0, 1.0, 0.0])
        );
        assert_eq!(ground.vertices[0].position[0], -GROUND_HALF_EXTENT);
        assert_eq!(ground.vertices[2].position[2], GROUND_HALF_EXTENT);
    }
}
