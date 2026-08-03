//! The Mask Tool: authoring washable goop masks on enemy actor models.
//!
//! # The effect being authored
//!
//! Washable goop is a per-pixel comparison the game runs on wired actors:
//!
//! ```text
//! visible = mask(goop UV) > K0_A
//! ```
//!
//! `K0_A` is a scalar the enemy's class drives (hit points, for StayPakkun),
//! sweeping from full coverage to clean as FLUDD washes it. The mask supplies
//! the *shape*: because the comparison is a hard threshold, texels cross it in
//! order of their mask value, so the coating recedes with a crisp edge tracing
//! the mask's gradient. Bright mask paint clings longest; dark clears first.
//! A separate colour texture supplies the goop's look. Both ride one UV set.
//!
//! Retail authored that UV as a **front projection normalised to bounds** --
//! measuring StayPakkun's real goop UV puts it exactly on the unit square, and
//! front and back of the body share it (the coating is symmetric, so sharing is
//! intended). [`front_projection_bounds`] reproduces that, and the preview
//! below projects the model the same way, so what you paint and what you see
//! agree by construction.
//!
//! # This module
//!
//! - An actor sampler over the **loaded stage's own hierarchy** -- the enemies
//!   placed in your level, not the whole catalog.
//! - A model preview rasterised on the CPU, front-projected.
//! - A **UV layer** switch: the body layer renders the model's own skin; the
//!   goop layer renders it coated, composited through the projected UV.
//! - **Generate** seeds example goop: a rainbow colour map and StayPakkun's
//!   retail 32x32 mask when the stage carries it, else a procedural stand-in.
//! - **Play full cycle** sweeps `K0_A` so the wash recedes exactly as in game.
//!
//! Painting strokes onto the mask, and writing the authored UV and mask back
//! into a model's material, are the phases after this one. The window is
//! rendered inside the inspector for now; the standalone BrawlBox-style window
//! (menu bar repurposed to Actor/Edit/View/Mask) is a presentation change over
//! the same state.

use super::*;

/// Which UV layer the preview composites through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MaskUvLayer {
    /// The model's own skin, through its body UV.
    Body,
    /// The goop mask layer, through the front-projected UV.
    Goop,
}

/// One placed enemy the Mask Tool can target.
struct MaskActorChoice {
    object_id: String,
    label: String,
    model_path: String,
}

/// A rasterised preview of one actor, front-projected.
///
/// Rasterising is done once per actor; sweeping the wash only re-evaluates the
/// per-pixel comparison, so the animation stays cheap.
pub(super) struct MaskPreview {
    object_id: String,
    width: usize,
    height: usize,
    /// Shaded body colour per pixel, or `None` where nothing was drawn.
    base: Vec<Option<[u8; 4]>>,
    /// Front-projected UV per pixel, in `[0, 1]`.
    goop_uv: Vec<[f32; 2]>,
    triangle_count: usize,
}

/// Projects points onto the front plane and normalises to the unit square.
///
/// This is Blender's "Project from View (Bounds)" and the layout retail's goop
/// UV uses: `x` and `y` become `u` and `v`, then the projected extent is fitted
/// to `[0, 1]` so the mask always covers the whole canvas. A degenerate axis
/// collapses to the middle of the range rather than dividing by zero.
pub(super) fn front_projection_bounds(points: &[[f32; 3]]) -> Vec<[f32; 2]> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for point in points {
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    let span = [max[0] - min[0], max[1] - min[1]];
    points
        .iter()
        .map(|point| {
            std::array::from_fn(|axis| {
                if span[axis] > f32::EPSILON {
                    ((point[axis] - min[axis]) / span[axis]).clamp(0.0, 1.0)
                } else {
                    0.5
                }
            })
        })
        .collect()
}

/// Whether goop shows at a texel: the game's own comparison.
pub(super) fn goop_is_visible(mask_value: u8, threshold: u8) -> bool {
    mask_value > threshold
}

/// A rainbow stand-in colour map, so a generated coating is obviously custom.
fn rainbow_goop(u: f32, v: f32) -> [u8; 4] {
    let hue = (u * 0.75 + v * 0.25).fract() * 6.0;
    let sector = hue.floor() as i32;
    let f = hue - hue.floor();
    let (r, g, b) = match sector.rem_euclid(6) {
        0 => (1.0, f, 0.0),
        1 => (1.0 - f, 1.0, 0.0),
        2 => (0.0, 1.0, f),
        3 => (0.0, 1.0 - f, 1.0),
        4 => (f, 0.0, 1.0),
        _ => (1.0, 0.0, 1.0 - f),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255]
}

/// A procedural mask used when the stage carries no retail mask to borrow:
/// a smooth blob field, so the wash still recedes in a legible shape.
fn procedural_mask(size: usize) -> (usize, Vec<u8>) {
    let mut values = vec![0u8; size * size];
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            let blobs = (u * 8.0).sin() * (v * 6.0).cos() + (u * 3.0 + v * 5.0).sin() * 0.6;
            let normalised = ((blobs + 1.6) / 3.2).clamp(0.0, 1.0);
            values[y * size + x] = (normalised * 255.0) as u8;
        }
    }
    (size, values)
}

impl SmsEditorApp {
    /// Enemy actors placed in the loaded stage, as sampler choices.
    fn mask_actor_choices(&self) -> Vec<MaskActorChoice> {
        let (Some(document), Some(registry)) = (self.document.as_ref(), self.registry.as_ref())
        else {
            return Vec::new();
        };
        let mut choices = Vec::new();
        for object in &document.objects {
            if registry.find_enemy_actor(&object.factory_name).is_none() {
                continue;
            }
            let Some(model_path) = document
                .actor_preview(object)
                .map(|preview| preview.model_path.clone())
            else {
                continue;
            };
            choices.push(MaskActorChoice {
                object_id: object.id.clone(),
                label: format!("{} \u{2014} {}", object.factory_name, object.id),
                model_path,
            });
        }
        choices.sort_by(|left, right| left.label.cmp(&right.label));
        choices
    }

    /// Rasterises the chosen actor's model, front-projected, into a preview.
    fn build_mask_preview(&mut self, choice: &MaskActorChoice) {
        const CANVAS: usize = 320;

        let Some(document) = self.document.as_ref() else {
            return;
        };
        let bytes = match document.read_asset_bytes(&choice.model_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.log
                    .push(format!("Mask Tool could not read the model: {error}"));
                return;
            }
        };
        let model = match sms_formats::J3dFile::parse(&bytes) {
            Ok(model) => model,
            Err(error) => {
                self.log
                    .push(format!("Mask Tool could not parse the model: {error}"));
                return;
            }
        };
        let geometry = match model.geometry_preview() {
            Ok(geometry) => geometry,
            Err(error) => {
                self.log
                    .push(format!("Mask Tool could not read the geometry: {error}"));
                return;
            }
        };
        if geometry.triangles.is_empty() {
            self.log
                .push("That model has no triangles to preview.".to_string());
            return;
        }

        // Front projection over every vertex, fitted to bounds -- the same
        // layout the generated goop UV uses.
        let corners = geometry
            .triangles
            .iter()
            .flat_map(|triangle| triangle.vertices)
            .collect::<Vec<_>>();
        let projected = front_projection_bounds(&corners);

        let mut base = vec![None; CANVAS * CANVAS];
        let mut goop_uv = vec![[0.0f32; 2]; CANVAS * CANVAS];
        let mut depth = vec![f32::INFINITY; CANVAS * CANVAS];

        for (index, triangle) in geometry.triangles.iter().enumerate() {
            let uv = [
                projected[index * 3],
                projected[index * 3 + 1],
                projected[index * 3 + 2],
            ];
            // Screen space: the projection, with v flipped so up is up.
            let screen: [[f32; 2]; 3] = std::array::from_fn(|corner| {
                [
                    uv[corner][0] * (CANVAS - 1) as f32,
                    (1.0 - uv[corner][1]) * (CANVAS - 1) as f32,
                ]
            });
            let z: [f32; 3] = std::array::from_fn(|corner| triangle.vertices[corner][2]);

            let min_x = screen
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as usize;
            let max_x = (screen
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as usize)
                .min(CANVAS - 1);
            let min_y = screen
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as usize;
            let max_y = (screen
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as usize)
                .min(CANVAS - 1);

            let area = (screen[1][0] - screen[0][0]) * (screen[2][1] - screen[0][1])
                - (screen[2][0] - screen[0][0]) * (screen[1][1] - screen[0][1]);
            if area.abs() < f32::EPSILON {
                continue;
            }

            // Flat shading from the face normal keeps the silhouette readable
            // without a lighting rig.
            let shade = triangle
                .normals
                .map(|normals| {
                    let n = normals[0];
                    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2])
                        .sqrt()
                        .max(f32::EPSILON);
                    (0.35 + 0.65 * (n[2] / length).abs()).clamp(0.0, 1.0)
                })
                .unwrap_or(0.8);

            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let point = [x as f32 + 0.5, y as f32 + 0.5];
                    let w0 = ((screen[1][0] - screen[0][0]) * (point[1] - screen[0][1])
                        - (point[0] - screen[0][0]) * (screen[1][1] - screen[0][1]))
                        / area;
                    let w1 = ((point[0] - screen[0][0]) * (screen[2][1] - screen[0][1])
                        - (screen[2][0] - screen[0][0]) * (point[1] - screen[0][1]))
                        / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let pixel_depth = z[0] * w2 + z[1] * w1 + z[2] * w0;
                    let slot = y * CANVAS + x;
                    if pixel_depth >= depth[slot] {
                        continue;
                    }
                    depth[slot] = pixel_depth;

                    let interpolated_uv = [
                        uv[0][0] * w2 + uv[1][0] * w1 + uv[2][0] * w0,
                        uv[0][1] * w2 + uv[1][1] * w1 + uv[2][1] * w0,
                    ];
                    goop_uv[slot] = interpolated_uv;

                    // Body colour: the material's own texture through UV0 when
                    // the model provides one, else flat shading.
                    let mut colour = [
                        (200.0 * shade) as u8,
                        (200.0 * shade) as u8,
                        (205.0 * shade) as u8,
                        255,
                    ];
                    if let Some(texture) = triangle
                        .material_index
                        .and_then(|material| geometry.materials.get(material))
                        .and_then(|material| material.texture_indices.iter().flatten().next())
                        .and_then(|index| geometry.textures.get(*index))
                    {
                        if let Some(set) = triangle.tex_coord_sets[0] {
                            let u = set[0][0] * w2 + set[1][0] * w1 + set[2][0] * w0;
                            let v = set[0][1] * w2 + set[1][1] * w1 + set[2][1] * w0;
                            if let Some(sample) = sample_texture(texture, u, v) {
                                colour = [
                                    (sample[0] as f32 * shade) as u8,
                                    (sample[1] as f32 * shade) as u8,
                                    (sample[2] as f32 * shade) as u8,
                                    255,
                                ];
                            }
                        }
                    }
                    base[slot] = Some(colour);
                }
            }
        }

        self.mask_preview = Some(MaskPreview {
            object_id: choice.object_id.clone(),
            width: CANVAS,
            height: CANVAS,
            base,
            goop_uv,
            triangle_count: geometry.triangles.len(),
        });
        self.log.push(format!(
            "Mask Tool loaded {} ({} triangles).",
            choice.label,
            geometry.triangles.len()
        ));
    }

    /// Seeds the example goop: rainbow colour, and the retail StayPakkun mask
    /// when this stage carries it.
    fn generate_mask_content(&mut self) {
        let borrowed = self.retail_polmask();
        let (size, values) = borrowed.unwrap_or_else(|| procedural_mask(32));
        self.mask_mask_size = size;
        self.mask_mask = values;
        self.mask_generated = true;
    }

    /// StayPakkun's 32x32 pollution mask, if the loaded stage carries pakun.bmd.
    fn retail_polmask(&mut self) -> Option<(usize, Vec<u8>)> {
        let document = self.document.as_ref()?;
        let candidates = document
            .assets
            .iter()
            .filter(|asset| {
                asset
                    .path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .replace('\\', "/")
                    .ends_with("pakun.bmd")
            })
            .map(|asset| asset.path.clone())
            .collect::<Vec<_>>();
        for path in candidates {
            let document = self.document.as_ref()?;
            let Ok(bytes) = document.read_asset_bytes(&path) else {
                continue;
            };
            let Ok(model) = sms_formats::J3dFile::parse(&bytes) else {
                continue;
            };
            let Ok(geometry) = model.geometry_preview() else {
                continue;
            };
            let Some(texture) = geometry
                .textures
                .iter()
                .find(|texture| texture.name.to_ascii_lowercase().contains("polmask"))
            else {
                continue;
            };
            let size = texture.width.min(texture.height) as usize;
            if size == 0 {
                continue;
            }
            let mut values = vec![0u8; size * size];
            for y in 0..size {
                for x in 0..size {
                    let source = (y * texture.width as usize + x) * 4;
                    values[y * size + x] = texture.rgba.get(source).copied().unwrap_or(0);
                }
            }
            self.log.push(format!(
                "Borrowed the retail mask '{}' ({size}x{size}).",
                texture.name
            ));
            return Some((size, values));
        }
        None
    }

    /// Composites the preview at the current wash phase into an image.
    fn mask_preview_image(&self) -> Option<egui::ColorImage> {
        let preview = self.mask_preview.as_ref()?;
        let threshold = (self.mask_wash_phase.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut pixels = Vec::with_capacity(preview.width * preview.height * 4);
        for slot in 0..preview.width * preview.height {
            let Some(base) = preview.base[slot] else {
                pixels.extend_from_slice(&[24, 24, 30, 255]);
                continue;
            };
            let mut colour = base;
            if self.mask_uv_layer == MaskUvLayer::Goop && self.mask_generated {
                let [u, v] = preview.goop_uv[slot];
                let size = self.mask_mask_size.max(1);
                let x = ((u * size as f32) as usize).min(size - 1);
                let y = (((1.0 - v) * size as f32) as usize).min(size - 1);
                let mask_value = self.mask_mask.get(y * size + x).copied().unwrap_or(0);
                if goop_is_visible(mask_value, threshold) {
                    let goop = rainbow_goop(u, v);
                    // Keep the model's shading under the coating so form reads.
                    let shade = base[0].max(base[1]).max(base[2]) as f32 / 255.0;
                    colour = [
                        (goop[0] as f32 * (0.45 + 0.55 * shade)) as u8,
                        (goop[1] as f32 * (0.45 + 0.55 * shade)) as u8,
                        (goop[2] as f32 * (0.45 + 0.55 * shade)) as u8,
                        255,
                    ];
                }
            }
            pixels.extend_from_slice(&colour);
        }
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [preview.width, preview.height],
            &pixels,
        ))
    }

    /// The inspector panel for the Mask Tool.
    pub(super) fn mask_tool_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Mask Tool");
        ui.label(
            egui::RichText::new("Author washable goop on the enemies placed in this stage")
                .small()
                .color(egui::Color32::GRAY),
        );
        ui.separator();

        let choices = self.mask_actor_choices();
        if choices.is_empty() {
            ui.label(
                "No placed enemy actor in this stage has a previewable model. Place one, or open \
                 a stage that has one.",
            );
            return;
        }

        let selected_label = self
            .mask_selected_actor
            .as_ref()
            .and_then(|id| choices.iter().find(|choice| &choice.object_id == id))
            .map(|choice| choice.label.clone())
            .unwrap_or_else(|| "Choose an actor".to_string());
        let mut picked = None;
        egui::ComboBox::from_label("Actor")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for choice in &choices {
                    let active = self.mask_selected_actor.as_deref() == Some(&choice.object_id);
                    if ui.selectable_label(active, &choice.label).clicked() {
                        picked = Some(choice.object_id.clone());
                    }
                }
            });
        if let Some(id) = picked {
            self.mask_selected_actor = Some(id.clone());
            if let Some(choice) = choices.iter().find(|choice| choice.object_id == id) {
                self.build_mask_preview(choice);
            }
        }

        if self.mask_selected_actor.is_none() {
            ui.separator();
            ui.label("Pick an actor to load its model.");
            return;
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("UV layer:");
            ui.selectable_value(&mut self.mask_uv_layer, MaskUvLayer::Body, "Body (UV0)");
            ui.selectable_value(&mut self.mask_uv_layer, MaskUvLayer::Goop, "Goop");
        });
        if self.mask_uv_layer == MaskUvLayer::Goop && !self.mask_generated {
            ui.colored_label(
                egui::Color32::from_rgb(255, 180, 90),
                "Generate the goop map to see the coating.",
            );
        }

        ui.horizontal(|ui| {
            if ui
                .button("Generate goop map + mask")
                .on_hover_text(
                    "Seed a rainbow colour map and the retail 32x32 StayPakkun mask (or a \
                     procedural stand-in if this stage has none).",
                )
                .clicked()
            {
                self.generate_mask_content();
                self.mask_uv_layer = MaskUvLayer::Goop;
            }
            if ui
                .button("Create goop UV (front projection)")
                .on_hover_text(
                    "The preview already projects the model this way -- retail's own goop UV is \
                     a front projection fitted to the [0,1] canvas.",
                )
                .clicked()
            {
                self.log.push(
                    "The preview's goop UV is the front projection; writing it into the model's \
                     material is the authoring phase."
                        .to_string(),
                );
            }
        });

        ui.separator();
        ui.heading("Brush");
        ui.add(
            egui::Slider::new(&mut self.mask_brush_radius, 1.0..=64.0)
                .text("Radius")
                .clamping(egui::SliderClamping::Always),
        );
        ui.add(
            egui::Slider::new(&mut self.mask_brush_opacity, 0.0..=1.0)
                .text("Opacity")
                .clamping(egui::SliderClamping::Always),
        );
        ui.label(
            egui::RichText::new("Painting strokes onto the mask is the next phase.")
                .small()
                .color(egui::Color32::GRAY),
        );

        ui.separator();
        ui.heading("Wash");
        self.mask_wash_controls(ui);

        ui.separator();
        if let Some(image) = self.mask_preview_image() {
            let texture = self.mask_texture.get_or_insert_with(|| {
                ui.ctx()
                    .load_texture("mask-tool-preview", image.clone(), Default::default())
            });
            texture.set(image, Default::default());
            let size = texture.size_vec2();
            ui.add(egui::Image::new(&*texture).fit_to_exact_size(size));
        }
        if let Some(preview) = self.mask_preview.as_ref() {
            ui.label(
                egui::RichText::new(format!(
                    "{} \u{2014} {} triangles, front-projected",
                    preview.object_id, preview.triangle_count
                ))
                .small()
                .color(egui::Color32::GRAY),
            );
        }
    }

    /// The wash-cycle preview: sweeps the threshold the mask is compared to.
    fn mask_wash_controls(&mut self, ui: &mut egui::Ui) {
        const CYCLE_SECONDS: f32 = 4.0;

        if self.mask_wash_playing {
            let dt = ui.input(|input| input.stable_dt).clamp(0.0, 0.1);
            self.mask_wash_phase -= dt / CYCLE_SECONDS;
            if self.mask_wash_phase <= 0.0 {
                self.mask_wash_phase = 0.0;
                self.mask_wash_playing = false;
            } else {
                ui.ctx().request_repaint();
            }
        }

        ui.horizontal(|ui| {
            let label = if self.mask_wash_playing {
                "Stop"
            } else {
                "Play full cycle"
            };
            if ui
                .button(label)
                .on_hover_text("Sweep the wash threshold from fully coated to clean")
                .clicked()
            {
                if self.mask_wash_playing {
                    self.mask_wash_playing = false;
                } else {
                    self.mask_wash_phase = 1.0;
                    self.mask_wash_playing = true;
                }
            }
            if ui.button("Reset").clicked() {
                self.mask_wash_playing = false;
                self.mask_wash_phase = 1.0;
            }
        });

        ui.add(
            egui::Slider::new(&mut self.mask_wash_phase, 0.0..=1.0)
                .text("Coverage")
                .clamping(egui::SliderClamping::Always),
        );
        ui.label(
            egui::RichText::new(format!(
                "threshold K0_A \u{2248} {}  (mask > this stays coated)",
                (self.mask_wash_phase * 255.0).round() as u16
            ))
            .small()
            .color(egui::Color32::GRAY),
        );
    }
}

/// Nearest-neighbour sample of a decoded preview texture, with wrapping.
fn sample_texture(texture: &sms_formats::J3dTexturePreview, u: f32, v: f32) -> Option<[u8; 4]> {
    let width = texture.width as usize;
    let height = texture.height as usize;
    if width == 0 || height == 0 || texture.rgba.len() < width * height * 4 {
        return None;
    }
    let x = ((u.rem_euclid(1.0)) * width as f32) as usize % width;
    let y = ((v.rem_euclid(1.0)) * height as f32) as usize % height;
    let base = (y * width + x) * 4;
    Some([
        texture.rgba[base],
        texture.rgba[base + 1],
        texture.rgba[base + 2],
        texture.rgba[base + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated UV must fill the canvas exactly, the way retail's does.
    #[test]
    fn front_projection_fits_the_unit_square() {
        let points = [
            [-100.0, 40.0, 5.0],
            [300.0, 40.0, -5.0],
            [100.0, 240.0, 0.0],
        ];
        let uv = front_projection_bounds(&points);
        let us = uv.iter().map(|p| p[0]).collect::<Vec<_>>();
        let vs = uv.iter().map(|p| p[1]).collect::<Vec<_>>();
        assert!(us.iter().cloned().fold(f32::INFINITY, f32::min).abs() < 1e-6);
        assert!((us.iter().cloned().fold(f32::NEG_INFINITY, f32::max) - 1.0).abs() < 1e-6);
        assert!(vs.iter().cloned().fold(f32::INFINITY, f32::min).abs() < 1e-6);
        assert!((vs.iter().cloned().fold(f32::NEG_INFINITY, f32::max) - 1.0).abs() < 1e-6);
    }

    /// A flat axis must not divide by zero; it collapses to the middle.
    #[test]
    fn a_degenerate_axis_collapses_instead_of_dividing_by_zero() {
        let points = [[10.0, 7.0, 0.0], [20.0, 7.0, 0.0]];
        let uv = front_projection_bounds(&points);
        assert!(uv.iter().all(|p| p[1] == 0.5));
        assert!(uv.iter().all(|p| p[0].is_finite()));
    }

    /// The wash is the game's comparison: bright mask clings, dark clears.
    #[test]
    fn goop_recedes_in_mask_order_as_the_threshold_sweeps() {
        let dark = 40u8;
        let bright = 200u8;
        // Fully coated: everything is above the floor.
        assert!(goop_is_visible(dark, 0));
        assert!(goop_is_visible(bright, 0));
        // Mid sweep: only the bright paint still clings.
        assert!(!goop_is_visible(dark, 128));
        assert!(goop_is_visible(bright, 128));
        // Clean: nothing survives the top of the range.
        assert!(!goop_is_visible(dark, 255));
        assert!(!goop_is_visible(bright, 255));
    }
}
