use super::*;

impl SmsEditorApp {
    pub(super) fn issue_counts(&self) -> (usize, usize) {
        let warnings = self
            .issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Warning)
            .count();
        let errors = self
            .issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Error)
            .count();
        (warnings, errors)
    }

    pub(super) fn object_screen_positions(
        &self,
        rect: egui::Rect,
    ) -> Vec<(String, egui::Pos2, String)> {
        self.document
            .as_ref()
            .map(|document| {
                document
                    .objects
                    .iter()
                    .filter_map(|object| {
                        self.project_world_to_screen(rect, object.transform.translation)
                            .map(|(screen, _)| {
                                (object.id.clone(), screen, object.factory_name.clone())
                            })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn world_to_screen(&self, rect: egui::Rect, point: [f32; 3]) -> egui::Pos2 {
        self.project_world_to_screen(rect, point)
            .map(|(screen, _)| screen)
            .unwrap_or(rect.center() + self.viewport_pan)
    }

    pub(super) fn project_world_to_screen(
        &self,
        rect: egui::Rect,
        point: [f32; 3],
    ) -> Option<(egui::Pos2, f32)> {
        let frame = self.camera_frame();
        let rel = vec3_sub(point, frame.position);
        let depth = vec3_dot(rel, frame.forward);
        if depth < VIEWPORT_NEAR_CLIP || !depth.is_finite() {
            return None;
        }

        let focal = perspective_focal_length(rect, self.viewport_zoom);
        let x = vec3_dot(rel, frame.right) / depth * focal;
        let y = vec3_dot(rel, frame.up) / depth * focal;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }

        Some((rect.center() + self.viewport_pan + egui::vec2(x, -y), depth))
    }

    pub(super) fn project_world_segment_to_screen(
        &self,
        rect: egui::Rect,
        start: [f32; 3],
        end: [f32; 3],
    ) -> Option<[egui::Pos2; 2]> {
        let [start, end] =
            clip_world_segment_to_near_plane(self.camera_frame(), start, end, VIEWPORT_NEAR_CLIP)?;
        Some([
            self.project_world_to_screen(rect, start)?.0,
            self.project_world_to_screen(rect, end)?.0,
        ])
    }

    pub(super) fn camera_frame(&self) -> CameraFrame {
        let camera = self.renderer.camera();
        let yaw = camera.yaw_degrees.to_radians();
        let pitch = camera.pitch_degrees.to_radians();
        let forward = vec3_normalize([
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ]);
        let right = vec3_normalize([-yaw.cos(), 0.0, yaw.sin()]);
        let up = vec3_normalize(vec3_cross(right, forward));
        let position = vec3_sub(camera.focus, vec3_scale(forward, camera.distance));
        CameraFrame {
            position,
            right,
            up,
            forward,
        }
    }

    pub(super) fn screen_to_world_floor(&self, rect: egui::Rect, pos: egui::Pos2) -> [f32; 3] {
        let frame = self.camera_frame();
        let floor_y = self.renderer.camera().focus[1];
        let focal = perspective_focal_length(rect, self.viewport_zoom);
        let local = pos - rect.center() - self.viewport_pan;
        let ray = vec3_normalize(vec3_add(
            frame.forward,
            vec3_add(
                vec3_scale(frame.right, local.x / focal),
                vec3_scale(frame.up, -local.y / focal),
            ),
        ));
        if ray[1].abs() < 0.0001 {
            return self.renderer.camera().focus;
        }
        let t = (floor_y - frame.position[1]) / ray[1];
        if !t.is_finite() || t <= 0.0 {
            return self.renderer.camera().focus;
        }

        vec3_add(frame.position, vec3_scale(ray, t))
    }
}
