use glam::{Mat3, Mat4, Quat, Vec3};
use thiserror::Error;

/// Opaque identifier for renderer-owned mesh data referenced by a scene node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MeshHandle(u32);

impl MeshHandle {
    /// Creates a mesh handle from an engine-owned mesh table index.
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the engine-owned mesh table index.
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Stable index for a node while it remains in its owning [`Scene`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SceneNodeId(usize);

impl SceneNodeId {
    /// Returns the node's scene-local index.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Position, orientation, and scale for a scene object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

impl Transform {
    /// Creates a validated transform and normalizes its quaternion.
    ///
    /// Zero or non-finite scales are rejected because they make normal transforms singular.
    pub fn from_trs(translation: Vec3, rotation: Quat, scale: Vec3) -> Option<Self> {
        let valid_translation = translation.is_finite();
        let valid_rotation = rotation.is_finite() && rotation.length_squared() > 1.0e-12;
        let valid_scale = scale.is_finite()
            && scale.x.abs() > 1.0e-6
            && scale.y.abs() > 1.0e-6
            && scale.z.abs() > 1.0e-6;
        if !(valid_translation && valid_rotation && valid_scale) {
            return None;
        }
        Some(Self {
            translation,
            rotation: rotation.normalize(),
            scale,
        })
    }

    /// Returns the identity transform.
    pub const fn identity() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Returns the translation component.
    pub const fn translation(self) -> Vec3 {
        self.translation
    }

    /// Returns the rotation component.
    pub const fn rotation(self) -> Quat {
        self.rotation
    }

    /// Returns the scale component.
    pub const fn scale(self) -> Vec3 {
        self.scale
    }

    /// Builds a column-major transform matrix in scale, rotation, translation order.
    pub fn matrix(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Transforms a local-space point to world space.
    pub fn transform_point(self, point: Vec3) -> Vec3 {
        self.matrix().transform_point3(point)
    }

    /// Transforms a local-space normal by the inverse-transpose normal matrix.
    pub fn transform_normal(self, normal: Vec3) -> Vec3 {
        let normal_matrix = Mat3::from_mat4(self.matrix()).inverse().transpose();
        (normal_matrix * normal).normalize_or_zero()
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

/// A scene node that binds a mesh handle to its transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneNode {
    mesh: MeshHandle,
    transform: Transform,
}

impl SceneNode {
    /// Returns the referenced mesh handle.
    pub const fn mesh(self) -> MeshHandle {
        self.mesh
    }

    /// Returns the node's local-to-world transform.
    pub const fn transform(self) -> Transform {
        self.transform
    }
}

/// Bound on the number of scene nodes retained by a scene.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("scene node limit reached ({limit})")]
pub struct SceneError {
    /// Maximum node count configured for the scene.
    pub limit: usize,
}

/// Bounded list of mesh instances used to build or update a render scene.
#[derive(Clone, Debug)]
pub struct Scene {
    nodes: Vec<SceneNode>,
    max_nodes: usize,
}

impl Scene {
    /// Creates an empty scene that retains no more than `max_nodes` objects.
    pub fn new(max_nodes: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(max_nodes.min(1024)),
            max_nodes,
        }
    }

    /// Adds a mesh instance unless this scene reached its configured capacity.
    pub fn add_node(
        &mut self,
        mesh: MeshHandle,
        transform: Transform,
    ) -> Result<SceneNodeId, SceneError> {
        if self.nodes.len() >= self.max_nodes {
            return Err(SceneError {
                limit: self.max_nodes,
            });
        }
        let id = SceneNodeId(self.nodes.len());
        self.nodes.push(SceneNode { mesh, transform });
        Ok(id)
    }

    /// Returns a node while its scene-local identifier is in range.
    pub fn node(&self, id: SceneNodeId) -> Option<&SceneNode> {
        self.nodes.get(id.0)
    }

    /// Changes a node transform without changing its stable scene-local identifier.
    pub fn set_transform(&mut self, id: SceneNodeId, transform: Transform) -> bool {
        let Some(node) = self.nodes.get_mut(id.0) else {
            return false;
        };
        node.transform = transform;
        true
    }

    /// Iterates nodes in insertion order, which can also define static submission order.
    pub fn nodes(&self) -> &[SceneNode] {
        &self.nodes
    }

    /// Configured node capacity.
    pub fn max_nodes(&self) -> usize {
        self.max_nodes
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new(4096)
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::{MeshHandle, Scene, Transform};

    #[test]
    fn transform_composes_scale_rotation_and_translation() {
        let transform = Transform::from_trs(
            Vec3::new(4.0, 2.0, -1.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::new(2.0, 1.0, 1.0),
        )
        .unwrap();
        let point = transform.transform_point(Vec3::X);

        assert!((point.x - 4.0).abs() < 0.0001);
        assert!((point.y - 2.0).abs() < 0.0001);
        assert!((point.z + 3.0).abs() < 0.0001);
    }

    #[test]
    fn normal_transform_uses_inverse_transpose_for_non_uniform_scale() {
        let transform =
            Transform::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::new(2.0, 1.0, 0.5)).unwrap();
        let normal = transform.transform_normal(Vec3::new(1.0, 1.0, 0.0));

        assert!(normal.is_normalized());
        assert!(normal.x < normal.y);
    }

    #[test]
    fn transform_rejects_singular_or_non_finite_components() {
        assert!(Transform::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO).is_none());
        assert!(
            Transform::from_trs(Vec3::splat(f32::INFINITY), Quat::IDENTITY, Vec3::ONE).is_none()
        );
    }

    #[test]
    fn scene_nodes_keep_mesh_transform_and_obey_capacity() {
        let mut scene = Scene::new(1);
        let node = scene
            .add_node(MeshHandle::new(7), Transform::identity())
            .unwrap();

        assert_eq!(scene.node(node).unwrap().mesh(), MeshHandle::new(7));
        assert!(
            scene
                .add_node(MeshHandle::new(8), Transform::identity())
                .is_err()
        );
        assert_eq!(scene.max_nodes(), 1);
    }
}
