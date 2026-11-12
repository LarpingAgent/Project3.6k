use glam::{Mat4, Vec3};

const FIELD_OF_VIEW_RADIANS: f32 = std::f32::consts::FRAC_PI_3;
const NEAR_PLANE: f32 = 0.05;
const FAR_PLANE: f32 = 500.0;
const MOVE_SPEED: f32 = 4.0;
const MAX_FRAME_DELTA_SECONDS: f32 = 0.1;
const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

/// First-person camera pose and right-handed Vulkan view-projection math.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    position: Vec3,
    yaw_radians: f32,
    pitch_radians: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 1.65, 4.5),
            yaw_radians: -std::f32::consts::FRAC_PI_2,
            pitch_radians: -0.27,
        }
    }
}

impl Camera {
    /// Returns the camera's world-space position.
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// Rotates by angular deltas in radians and clamps pitch away from the poles.
    pub fn rotate(&mut self, yaw_delta: f32, pitch_delta: f32) {
        if yaw_delta.is_finite() {
            self.yaw_radians = (self.yaw_radians + yaw_delta).rem_euclid(std::f32::consts::TAU);
        }
        if pitch_delta.is_finite() {
            self.pitch_radians =
                (self.pitch_radians - pitch_delta).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }
    }

    /// Moves along the camera's ground plane and world up axis.
    ///
    /// The combined input is normalized so diagonal movement is not faster. Large frame gaps
    /// are capped to keep movement stable after window dragging or a debugger pause.
    pub fn move_local(&mut self, forward: f32, right: f32, vertical: f32, delta_seconds: f32) {
        if !delta_seconds.is_finite() || delta_seconds <= 0.0 {
            return;
        }
        let forward = finite_axis(forward);
        let right = finite_axis(right);
        let vertical = finite_axis(vertical);
        let yaw_forward = Vec3::new(self.yaw_radians.cos(), 0.0, self.yaw_radians.sin());
        let yaw_right = yaw_forward.cross(Vec3::Y).normalize();
        let direction =
            (yaw_forward * forward + yaw_right * right + Vec3::Y * vertical).normalize_or_zero();
        let delta = delta_seconds.min(MAX_FRAME_DELTA_SECONDS);
        self.position += direction * MOVE_SPEED * delta;
    }

    /// Builds a right-handed view-projection matrix with Vulkan's [0, 1] depth and Y-down NDC.
    pub fn view_projection(&self, aspect_ratio: f32) -> Mat4 {
        let safe_aspect = if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
            aspect_ratio
        } else {
            1.0
        };
        let direction = Vec3::new(
            self.yaw_radians.cos() * self.pitch_radians.cos(),
            self.pitch_radians.sin(),
            self.yaw_radians.sin() * self.pitch_radians.cos(),
        )
        .normalize();
        let view = glam::camera::rh::view::look_to_mat4(self.position, direction, Vec3::Y);
        let projection = glam::camera::rh::proj::vulkan::perspective(
            FIELD_OF_VIEW_RADIANS,
            safe_aspect,
            NEAR_PLANE,
            FAR_PLANE,
        );
        projection * view
    }
}

fn finite_axis(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::Camera;
    use glam::Vec3;

    #[test]
    fn default_view_projects_forward_world_points_into_vulkan_depth_range() {
        let camera = Camera::default();
        let direction = Vec3::new(
            camera.yaw_radians.cos() * camera.pitch_radians.cos(),
            camera.pitch_radians.sin(),
            camera.yaw_radians.sin() * camera.pitch_radians.cos(),
        );
        let center = camera
            .view_projection(16.0 / 9.0)
            .project_point3(camera.position() + direction * 10.0);

        assert!(center.x.abs() < 0.0001);
        assert!(center.y.abs() < 0.0001);
        assert!((0.0..1.0).contains(&center.z));
    }

    #[test]
    fn invalid_aspect_ratio_falls_back_to_square_projection() {
        let camera = Camera::default();
        assert_eq!(camera.view_projection(0.0), camera.view_projection(1.0));
        assert_eq!(
            camera.view_projection(f32::NAN),
            camera.view_projection(1.0)
        );
    }

    #[test]
    fn movement_is_normalized_and_frame_delta_is_capped() {
        let mut camera = Camera::default();
        let start = camera.position();
        camera.move_local(1.0, 1.0, 0.0, 1.0);
        let moved = camera.position() - start;

        assert!((moved.length() - 0.4).abs() < 0.0001);
    }

    #[test]
    fn non_finite_input_does_not_move_camera() {
        let mut camera = Camera::default();
        let start = camera.position();
        camera.move_local(f32::NAN, f32::INFINITY, 0.0, 0.05);

        assert_eq!(camera.position(), start);
    }

    #[test]
    fn pitch_remains_finite_after_large_mouse_delta() {
        let mut camera = Camera::default();
        camera.rotate(0.0, 1.0e6);
        let point = camera
            .view_projection(1.0)
            .project_point3(camera.position() + Vec3::Z);

        assert!(point.is_finite());
    }
}
