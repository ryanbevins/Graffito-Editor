import io

# --------------------------------------------------- honest hue test bounds
r = "apps/sms-editor/src/tests.rs"
v = io.open(r, encoding="utf-8").read()
old = """        for start in [
            [0.8, 0.2, 0.2, 1.0],
            [0.2, 0.6, 0.9, 1.0],
            [0.35, 0.35, 0.35, 1.0],
        ] {"""
new = """        // Kept away from the edge of the gamut on purpose. A saturated colour
        // rotates to a negative channel, and clamping that back into range is
        // what moves its luminance -- the rotation is exact, staying in gamut
        // is not.
        for start in [
            [0.60, 0.45, 0.50, 1.0],
            [0.45, 0.55, 0.50, 1.0],
            [0.35, 0.35, 0.35, 1.0],
        ] {"""
assert v.count(old) == 1, "hue fixture anchor"
v = v.replace(old, new, 1)
v = v.replace("                (luma(color) - luma(start)).abs() < 0.02,",
              "                (luma(color) - luma(start)).abs() < 0.005,", 1)
io.open(r, "w", encoding="utf-8").write(v)

# ------------------------------------------------------- the winding repair
p = "apps/sms-editor/src/vertex_paint.rs"
s = io.open(p, encoding="utf-8").read()

repair = '''
    /// Rewinds triangles whose winding disagrees with their own vertex normals.
    ///
    /// A mesh can arrive with its faces wound against its normals, and baking
    /// as terrain is where it shows: the surface is drawn from the other side
    /// and reads as inside out. The normals are the intent -- they say which
    /// way the artist meant the surface to face -- so the winding is what gets
    /// corrected, and only on the triangles that actually disagree. A mesh
    /// that is already consistent is left exactly as it was.
    pub(super) fn repair_terrain_winding(&mut self) {
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log.push(
                "Select a terrain instance in the hierarchy before repairing winding."
                    .to_string(),
            );
            return;
        }
        let mut flipped = 0usize;
        let mut checked = 0usize;
        for target in &mut targets {
            for mesh in &mut target.document.meshes {
                for primitive in &mut mesh.primitives {
                    for face in primitive.indices.chunks_exact_mut(3) {
                        let corners = [face[0] as usize, face[1] as usize, face[2] as usize];
                        let Some(positions) = corners
                            .iter()
                            .map(|index| primitive.positions.get(*index).copied())
                            .collect::<Option<Vec<_>>>()
                        else {
                            continue;
                        };
                        let geometric = vec3_cross(
                            vec3_sub(positions[1], positions[0]),
                            vec3_sub(positions[2], positions[0]),
                        );
                        // A sliver has no reliable facing, so leave it be
                        // rather than flip it on rounding noise.
                        if vec3_dot(geometric, geometric) <= 1e-8 {
                            continue;
                        }
                        let intended = corners
                            .iter()
                            .filter_map(|index| primitive.normals.get(*index))
                            .fold([0.0f32; 3], |sum, normal| vec3_add(sum, *normal));
                        if vec3_dot(intended, intended) <= 1e-8 {
                            continue;
                        }
                        checked += 1;
                        if vec3_dot(geometric, intended) < 0.0 {
                            face.swap(1, 2);
                            flipped += 1;
                        }
                    }
                }
            }
        }

        if flipped == 0 {
            self.log.push(format!(
                "Winding already agrees with the normals across {checked} triangle(s); nothing \\
                 to repair."
            ));
            return;
        }
        // Geometry only, so the colour channel switch is left alone.
        self.commit_terrain_targets(targets, "Repaired terrain winding", false);
        self.log.push(format!(
            "Rewound {flipped} of {checked} triangle(s) to face the way their normals point."
        ));
    }

'''
anchor = "    /// Resets every terrain vertex to opaque white."
if s.count(anchor) != 1:
    anchor = "    pub(super) fn clear_terrain_vertex_colors(&mut self) {"
assert s.count(anchor) == 1, "repair anchor"
s = s.replace(anchor, repair.lstrip("\n") + anchor, 1)

old = """        if ui
            .button("Clear Paint")"""
new = """        if ui
            .button("Fix Facing")
            .on_hover_text(
                "Rewind triangles that face away from their own normals, which is what makes a \\
                 mesh look inside out once it bakes as terrain",
            )
            .clicked()
        {
            self.repair_terrain_winding();
        }
        if ui
            .button("Clear Paint")"""
assert s.count(old) == 1, "button anchor"
s = s.replace(old, new, 1)
io.open(p, "w", encoding="utf-8").write(s)
print("winding repair added")
