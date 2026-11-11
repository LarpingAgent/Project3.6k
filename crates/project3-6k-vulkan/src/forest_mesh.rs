use glam::{Quat, Vec3};
use project3_6k_core::{MeshHandle, Scene, SceneNode, Transform};

use super::{Result, VulkanRendererError};

const FOREST_PLANE_SCALE: [f32; 2] = [2.2581, 1.2702];
const TREE_DEPTHS: [(u32, f32); 5] = [(1, 0.0), (2, 0.1), (3, 0.25), (4, 0.1), (5, -0.45)];

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct ForestGpuVertex {
    pub(super) position: [f32; 3],
    color: [f32; 3],
    uv: [f32; 2],
    normal: [f32; 3],
}

#[derive(Debug)]
pub(super) struct ForestGeometry {
    pub(super) scene: Scene,
    pub(super) vertices: Vec<ForestGpuVertex>,
    pub(super) indices: Vec<u16>,
    pub(super) draw_ranges: Vec<(u32, u32)>,
}

#[derive(Clone, Copy)]
struct SourceVertex {
    position: [f32; 2],
    color: [f32; 3],
}

#[derive(Clone, Copy)]
struct MeshPoint {
    position: Vec3,
    color: [f32; 3],
    uv: [f32; 2],
}

struct ForestPolygon {
    mesh_index: u32,
    boundary: &'static [usize],
    triangles: &'static [[usize; 3]],
    half_depth: f32,
}

const SOURCE_VERTICES: [SourceVertex; 31] = [
    // Forest floor panel.
    SourceVertex {
        position: [-1.0, -0.56],
        color: [0.09, 0.22, 0.11],
    },
    SourceVertex {
        position: [1.0, -0.56],
        color: [0.09, 0.22, 0.11],
    },
    SourceVertex {
        position: [1.0, -1.0],
        color: [0.035, 0.12, 0.065],
    },
    SourceVertex {
        position: [-1.0, -1.0],
        color: [0.035, 0.12, 0.065],
    },
    // Distant ridge.
    SourceVertex {
        position: [-1.0, -0.38],
        color: [0.08, 0.19, 0.14],
    },
    SourceVertex {
        position: [-0.52, 0.3],
        color: [0.08, 0.19, 0.14],
    },
    SourceVertex {
        position: [-0.12, -0.38],
        color: [0.08, 0.19, 0.14],
    },
    SourceVertex {
        position: [-0.38, -0.38],
        color: [0.07, 0.17, 0.12],
    },
    SourceVertex {
        position: [0.12, 0.43],
        color: [0.07, 0.17, 0.12],
    },
    SourceVertex {
        position: [0.62, -0.38],
        color: [0.07, 0.17, 0.12],
    },
    // Left tree canopy and trunk.
    SourceVertex {
        position: [-0.84, -0.06],
        color: [0.12, 0.34, 0.16],
    },
    SourceVertex {
        position: [-0.48, -0.06],
        color: [0.12, 0.34, 0.16],
    },
    SourceVertex {
        position: [-0.66, 0.62],
        color: [0.2, 0.46, 0.2],
    },
    SourceVertex {
        position: [-0.71, -0.07],
        color: [0.3, 0.19, 0.1],
    },
    SourceVertex {
        position: [-0.61, -0.07],
        color: [0.3, 0.19, 0.1],
    },
    SourceVertex {
        position: [-0.61, -0.56],
        color: [0.2, 0.12, 0.07],
    },
    SourceVertex {
        position: [-0.71, -0.56],
        color: [0.2, 0.12, 0.07],
    },
    // Center tree.
    SourceVertex {
        position: [-0.22, -0.02],
        color: [0.09, 0.29, 0.13],
    },
    SourceVertex {
        position: [0.22, -0.02],
        color: [0.09, 0.29, 0.13],
    },
    SourceVertex {
        position: [0.0, 0.86],
        color: [0.17, 0.42, 0.18],
    },
    SourceVertex {
        position: [-0.055, -0.03],
        color: [0.32, 0.2, 0.1],
    },
    SourceVertex {
        position: [0.055, -0.03],
        color: [0.32, 0.2, 0.1],
    },
    SourceVertex {
        position: [0.055, -0.56],
        color: [0.21, 0.13, 0.07],
    },
    SourceVertex {
        position: [-0.055, -0.56],
        color: [0.21, 0.13, 0.07],
    },
    // Right tree.
    SourceVertex {
        position: [0.45, -0.04],
        color: [0.11, 0.31, 0.14],
    },
    SourceVertex {
        position: [0.81, -0.04],
        color: [0.11, 0.31, 0.14],
    },
    SourceVertex {
        position: [0.63, 0.62],
        color: [0.19, 0.43, 0.18],
    },
    SourceVertex {
        position: [0.58, -0.05],
        color: [0.3, 0.19, 0.1],
    },
    SourceVertex {
        position: [0.68, -0.05],
        color: [0.3, 0.19, 0.1],
    },
    SourceVertex {
        position: [0.68, -0.56],
        color: [0.2, 0.12, 0.07],
    },
    SourceVertex {
        position: [0.58, -0.56],
        color: [0.2, 0.12, 0.07],
    },
];

const POLYGONS: [ForestPolygon; 8] = [
    ForestPolygon {
        mesh_index: 1,
        boundary: &[0, 1, 2, 3],
        triangles: &[[0, 1, 2], [0, 2, 3]],
        half_depth: 0.09,
    },
    ForestPolygon {
        mesh_index: 2,
        boundary: &[10, 11, 12],
        triangles: &[[10, 11, 12]],
        half_depth: 0.16,
    },
    ForestPolygon {
        mesh_index: 2,
        boundary: &[13, 14, 15, 16],
        triangles: &[[13, 14, 15], [13, 15, 16]],
        half_depth: 0.11,
    },
    ForestPolygon {
        mesh_index: 3,
        boundary: &[17, 18, 19],
        triangles: &[[17, 18, 19]],
        half_depth: 0.18,
    },
    ForestPolygon {
        mesh_index: 3,
        boundary: &[20, 21, 22, 23],
        triangles: &[[20, 21, 22], [20, 22, 23]],
        half_depth: 0.11,
    },
    ForestPolygon {
        mesh_index: 4,
        boundary: &[24, 25, 26],
        triangles: &[[24, 25, 26]],
        half_depth: 0.16,
    },
    ForestPolygon {
        mesh_index: 4,
        boundary: &[27, 28, 29, 30],
        triangles: &[[27, 28, 29], [27, 29, 30]],
        half_depth: 0.11,
    },
    ForestPolygon {
        mesh_index: 5,
        boundary: &[4, 5, 6],
        triangles: &[[4, 5, 6]],
        half_depth: 0.07,
    },
];

const RIDGE_RIGHT: ForestPolygon = ForestPolygon {
    mesh_index: 5,
    boundary: &[7, 8, 9],
    triangles: &[[7, 8, 9]],
    half_depth: 0.07,
};

/// Creates the bounded scene representation and a one-time indexed mesh upload.
pub(super) fn build_forest_geometry() -> Result<ForestGeometry> {
    let scene = create_forest_scene()?;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut draw_ranges = Vec::with_capacity(scene.nodes().len());

    for node in scene.nodes() {
        let first_index =
            u32::try_from(indices.len()).map_err(|_| VulkanRendererError::MeshTooLarge)?;
        if node.mesh().index() == 0 {
            append_ground_plane(*node, &mut vertices, &mut indices)?;
        } else {
            for polygon in POLYGONS.iter().chain(std::iter::once(&RIDGE_RIGHT)) {
                if polygon.mesh_index == node.mesh().index() {
                    append_prism(*node, polygon, &mut vertices, &mut indices)?;
                }
            }
        }
        let index_count = u32::try_from(indices.len())
            .map_err(|_| VulkanRendererError::MeshTooLarge)?
            - first_index;
        draw_ranges.push((first_index, index_count));
    }

    Ok(ForestGeometry {
        scene,
        vertices,
        indices,
        draw_ranges,
    })
}

fn create_forest_scene() -> Result<Scene> {
    let mut scene = Scene::new(6);
    let ground = Transform::from_trs(
        // Tree and backdrop silhouettes meet near y=-0.71 in world space.
        Vec3::new(0.0, -0.71, -3.0),
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        Vec3::splat(12.0),
    )
    .expect("the ground transform is finite and non-singular");
    scene.add_node(MeshHandle::new(0), ground)?;

    let scale = Vec3::new(FOREST_PLANE_SCALE[0], FOREST_PLANE_SCALE[1], 1.0);
    for (mesh_index, depth) in TREE_DEPTHS {
        let transform = Transform::from_trs(Vec3::new(0.0, 0.0, depth), Quat::IDENTITY, scale)
            .expect("the fixed forest transforms are finite and non-singular");
        scene.add_node(MeshHandle::new(mesh_index), transform)?;
    }
    Ok(scene)
}

fn append_ground_plane(
    node: SceneNode,
    vertices: &mut Vec<ForestGpuVertex>,
    indices: &mut Vec<u16>,
) -> Result<()> {
    let color = [0.09, 0.22, 0.11];
    let corners = [
        SourceVertex {
            position: [-1.0, -1.0],
            color,
        },
        SourceVertex {
            position: [1.0, -1.0],
            color,
        },
        SourceVertex {
            position: [1.0, 1.0],
            color,
        },
        SourceVertex {
            position: [-1.0, 1.0],
            color,
        },
    ];
    for triangle in [[0, 1, 2], [0, 2, 3]] {
        let points = triangle.map(|index| {
            let source = corners[index];
            MeshPoint {
                position: Vec3::new(source.position[0], source.position[1], 0.0),
                color: source.color,
                uv: [
                    (source.position[0] + 1.0) * 6.0,
                    (source.position[1] + 1.0) * 6.0,
                ],
            }
        });
        push_triangle(node, Vec3::Z, points, vertices, indices)?;
    }
    Ok(())
}

fn append_prism(
    node: SceneNode,
    polygon: &ForestPolygon,
    vertices: &mut Vec<ForestGpuVertex>,
    indices: &mut Vec<u16>,
) -> Result<()> {
    for triangle in polygon.triangles {
        let front = triangle.map(|index| source_point(SOURCE_VERTICES[index], polygon.half_depth));
        push_triangle(node, Vec3::Z, front, vertices, indices)?;
        let back = [triangle[2], triangle[1], triangle[0]]
            .map(|index| source_point(SOURCE_VERTICES[index], -polygon.half_depth));
        push_triangle(node, -Vec3::Z, back, vertices, indices)?;
    }

    let center = polygon.boundary.iter().fold(Vec3::ZERO, |sum, index| {
        let position = SOURCE_VERTICES[*index].position;
        sum + Vec3::new(position[0], position[1], 0.0)
    }) / polygon.boundary.len() as f32;
    for edge in 0..polygon.boundary.len() {
        let start = SOURCE_VERTICES[polygon.boundary[edge]];
        let end = SOURCE_VERTICES[polygon.boundary[(edge + 1) % polygon.boundary.len()]];
        let start_position = Vec3::new(start.position[0], start.position[1], 0.0);
        let end_position = Vec3::new(end.position[0], end.position[1], 0.0);
        let edge_vector = end_position - start_position;
        let mut normal = Vec3::new(edge_vector.y, -edge_vector.x, 0.0).normalize_or_zero();
        let midpoint = (start_position + end_position) * 0.5;
        if normal.dot(midpoint - center) < 0.0 {
            normal = -normal;
        }
        let u = edge_vector.length() * 4.0;
        let side = [
            MeshPoint {
                position: start_position + Vec3::Z * polygon.half_depth,
                color: start.color,
                uv: [0.0, 0.0],
            },
            MeshPoint {
                position: end_position + Vec3::Z * polygon.half_depth,
                color: end.color,
                uv: [u, 0.0],
            },
            MeshPoint {
                position: end_position - Vec3::Z * polygon.half_depth,
                color: end.color,
                uv: [u, 1.0],
            },
            MeshPoint {
                position: start_position - Vec3::Z * polygon.half_depth,
                color: start.color,
                uv: [0.0, 1.0],
            },
        ];
        push_triangle(node, normal, [side[0], side[1], side[2]], vertices, indices)?;
        push_triangle(node, normal, [side[0], side[2], side[3]], vertices, indices)?;
    }
    Ok(())
}

fn source_point(source: SourceVertex, depth: f32) -> MeshPoint {
    MeshPoint {
        position: Vec3::new(source.position[0], source.position[1], depth),
        color: source.color,
        uv: [
            (source.position[0] + 1.0) * 4.0,
            (1.0 - source.position[1]) * 4.0,
        ],
    }
}

fn push_triangle(
    node: SceneNode,
    normal: Vec3,
    points: [MeshPoint; 3],
    vertices: &mut Vec<ForestGpuVertex>,
    indices: &mut Vec<u16>,
) -> Result<()> {
    let world_normal = node.transform().transform_normal(normal);
    for point in points {
        let index = u16::try_from(vertices.len()).map_err(|_| VulkanRendererError::MeshTooLarge)?;
        vertices.push(ForestGpuVertex {
            position: node.transform().transform_point(point.position).to_array(),
            color: point.color,
            uv: point.uv,
            normal: world_normal.to_array(),
        });
        indices.push(index);
    }
    Ok(())
}

pub(super) fn vertex_bytes(values: &[ForestGpuVertex]) -> &[u8] {
    // SAFETY: The repr(C) vertex contains eleven initialized f32 values and no padding bytes.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

pub(super) fn index_bytes(values: &[u16]) -> &[u8] {
    // SAFETY: Every u16 is initialized and has no padding; the slice is bounded by the vector.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

#[cfg(test)]
mod tests {
    use super::build_forest_geometry;

    #[test]
    fn forest_geometry_has_bounded_nodes_and_complete_index_ranges() {
        let geometry = build_forest_geometry().unwrap();
        assert_eq!(geometry.scene.max_nodes(), 6);
        assert_eq!(geometry.scene.nodes().len(), 6);
        assert_eq!(geometry.indices.len() % 3, 0);
        assert!(
            geometry
                .indices
                .iter()
                .all(|index| (*index as usize) < geometry.vertices.len())
        );

        let mut covered = vec![false; geometry.indices.len()];
        for (first_index, index_count) in &geometry.draw_ranges {
            assert_eq!(index_count % 3, 0);
            let end = (*first_index + *index_count) as usize;
            for slot in covered.iter_mut().take(end).skip(*first_index as usize) {
                assert!(!*slot);
                *slot = true;
            }
        }
        assert!(covered.into_iter().all(|slot| slot));
    }

    #[test]
    fn forest_mesh_contains_a_horizontal_ground_plane_and_volumetric_tree_sides() {
        let geometry = build_forest_geometry().unwrap();
        assert!(geometry.vertices.iter().any(|vertex| {
            (vertex.position[1] + 0.71).abs() < 0.0001 && vertex.normal[1] > 0.99
        }));
        assert!(geometry.vertices.iter().any(|vertex| {
            vertex.normal[2].abs() < 0.01 && (vertex.normal[0].abs() + vertex.normal[1].abs()) > 0.5
        }));
    }

    #[test]
    fn tree_trunks_reach_the_ground_plane_height() {
        let geometry = build_forest_geometry().unwrap();
        for mesh_index in 2..=4 {
            let (first_index, index_count) = geometry.draw_ranges[mesh_index as usize];
            let first = first_index as usize;
            let end = first + index_count as usize;
            let lowest_y = geometry.indices[first..end]
                .iter()
                .map(|index| geometry.vertices[*index as usize].position[1])
                .fold(f32::INFINITY, f32::min);
            assert!(
                (lowest_y + 0.71).abs() < 0.002,
                "tree mesh {mesh_index} bottomed at {lowest_y}"
            );
        }
    }

    #[test]
    fn far_ridge_is_submitted_after_near_trees_but_sits_at_farther_depth() {
        let geometry = build_forest_geometry().unwrap();
        let ridge = geometry.scene.nodes().last().unwrap();
        let center_tree = geometry
            .scene
            .nodes()
            .iter()
            .find(|node| node.mesh().index() == 3)
            .unwrap();
        assert_eq!(ridge.mesh().index(), 5);
        assert!(ridge.transform().translation().z < center_tree.transform().translation().z);
    }
}
