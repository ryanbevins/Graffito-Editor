use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use sms_formats::{
    mount_scene_archive, read_stage_asset_bytes, BmpFile, J3dFile, J3dRebuildDocument,
    J3dRebuildSectionData, StageAssetKind, YmpDocument, YmpLayer,
};
use sms_scene::{
    generate_floor_depth_map, generate_floor_pollution_model, whole_terrain_region,
    GoopAuthoringDocument, GoopBehavior, GoopLayerAuthoring, GoopLayerOrigin, GoopPlane,
    GoopRenderTriangle, GoopStyleSource, GoopTerrainTriangle, SourceFreeStageArchive,
    StageArchiveEdits, GOOP_AUTHORING_FORMAT_VERSION, GOOP_CELL_SIZE, GOOP_MAX_LAYERS,
};

use crate::camera::CameraProjection;

use super::*;

#[derive(Debug, Clone)]
pub(super) struct RetailGoopTemplate {
    pub(super) stage_id: String,
    pub(super) archive_path: PathBuf,
    pub(super) model_asset_path: PathBuf,
    pub(super) resource_stem: String,
    pub(super) layer_index: usize,
    pub(super) behavior: GoopBehavior,
    pub(super) compatible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GoopPixelSpan {
    start: usize,
    before: Vec<u8>,
    after: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum GoopUndoRecord {
    Pixels {
        layer: usize,
        spans: Vec<GoopPixelSpan>,
    },
    Snapshot(Box<GoopSnapshotUndo>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GoopSnapshotUndo {
    before: Option<GoopAuthoringDocument>,
    after: Option<GoopAuthoringDocument>,
    before_edits: StageArchiveEdits,
    after_edits: StageArchiveEdits,
}

#[derive(Debug, Clone)]
pub(super) struct GoopStroke {
    layer: usize,
    changed: BTreeMap<usize, (u8, u8)>,
    last_world: Option<[f32; 3]>,
}

pub(super) struct GoopRebuildOutcome {
    base_root: String,
    stage_id: String,
    terrain_fingerprint: u64,
    before: Option<GoopAuthoringDocument>,
    before_edits: StageArchiveEdits,
    document: StageDocument,
}

#[derive(Debug, Clone)]
struct FinalGoopTerrainSnapshot {
    collision_triangles: Vec<GoopTerrainTriangle>,
    render_triangles: Vec<GoopRenderTriangle>,
    fingerprint: u64,
}

pub(super) fn index_retail_goop_templates(
    archives: &[SceneArchiveInfo],
) -> (Vec<RetailGoopTemplate>, Vec<String>) {
    let mut templates = Vec::new();
    let mut warnings = Vec::new();
    for archive in archives.iter().filter(|archive| archive.path.is_file()) {
        let assets = match mount_scene_archive(&archive.path) {
            Ok(assets) => assets,
            Err(error) => {
                warnings.push(format!(
                    "Could not inspect goop templates in '{}': {error}",
                    archive.path.display()
                ));
                continue;
            }
        };
        let Some(ymp_asset) = assets.iter().find(|asset| {
            archive_resource_path(&asset.path)
                .is_some_and(|path| path.eq_ignore_ascii_case("map/ymap.ymp"))
        }) else {
            continue;
        };
        let ymp = match read_stage_asset_bytes(&ymp_asset.path).and_then(YmpDocument::parse) {
            Ok(ymp) => ymp,
            Err(error) => {
                warnings.push(format!(
                    "Could not parse {} ymap.ymp: {error}",
                    archive.stage_id
                ));
                continue;
            }
        };
        for (layer_index, layer) in ymp.layers.iter().enumerate() {
            if GoopPlane::from_runtime_code(layer.flags) != Some(GoopPlane::Floor) {
                continue;
            }
            let expected_stem = source_pollution_stem(layer_index, &archive.stage_id);
            let Some(model_asset) = assets.iter().find(|asset| {
                asset.kind == StageAssetKind::Model
                    && archive_resource_path(&asset.path).is_some_and(|path| {
                        Path::new(&path)
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .is_some_and(|stem| stem.eq_ignore_ascii_case(&expected_stem))
                            && path.to_ascii_lowercase().starts_with("map/pollution/")
                    })
            }) else {
                continue;
            };
            let model = match read_stage_asset_bytes(&model_asset.path)
                .and_then(J3dRebuildDocument::parse)
            {
                Ok(model) => model,
                Err(error) => {
                    warnings.push(format!(
                        "Blocked {} layer {} goop template: {error}",
                        archive.stage_id, layer_index
                    ));
                    continue;
                }
            };
            let has_material = model
                .sections
                .iter()
                .any(|section| matches!(section.data, J3dRebuildSectionData::Materials(_)));
            let first_texture_format = model.sections.iter().find_map(|section| {
                if let J3dRebuildSectionData::Textures(textures) = &section.data {
                    textures.textures.first().map(|texture| texture.format)
                } else {
                    None
                }
            });
            let Some(first_texture_format) = first_texture_format.filter(|_| has_material) else {
                warnings.push(format!(
                    "Blocked {} layer {} goop template because MAT3 or TEX1 texture zero is missing",
                    archive.stage_id, layer_index
                ));
                continue;
            };
            if first_texture_format != 1 {
                warnings.push(format!(
                    "Blocked {} layer {} goop template because texture zero is not mutable I8",
                    archive.stage_id, layer_index
                ));
                continue;
            }
            let texture_zero_is_bound = model
                .to_bytes()
                .and_then(J3dFile::parse)
                .and_then(|file| {
                    file.geometry_preview_with_loader_flags(SMS_POLLUTION_MODEL_LOAD_FLAGS)
                })
                .is_ok_and(|preview| {
                    preview.triangles.iter().any(|triangle| {
                        triangle.texture_index == Some(0) || triangle.mask_texture_index == Some(0)
                    })
                });
            if !texture_zero_is_bound {
                warnings.push(format!(
                    "Blocked {} layer {} goop template because its material does not bind texture zero",
                    archive.stage_id, layer_index
                ));
                continue;
            }
            templates.push(RetailGoopTemplate {
                stage_id: archive.stage_id.clone(),
                archive_path: archive.path.clone(),
                model_asset_path: model_asset.path.clone(),
                resource_stem: expected_stem,
                layer_index,
                behavior: GoopBehavior::from_runtime_code(layer.layer_type),
                compatible: true,
            });
        }
    }
    templates.sort_by(|left, right| {
        right
            .compatible
            .cmp(&left.compatible)
            .then_with(|| left.stage_id.cmp(&right.stage_id))
            .then_with(|| left.layer_index.cmp(&right.layer_index))
    });
    (templates, warnings)
}

fn archive_resource_path(path: &Path) -> Option<String> {
    path.to_string_lossy()
        .split_once("!/")
        .map(|(_, resource)| resource.replace('\\', "/"))
}

fn source_pollution_stem(index: usize, stage_id: &str) -> String {
    if stage_id.to_ascii_lowercase().starts_with("mare") {
        match index {
            7 => return "pollutionA".to_string(),
            8 => return "pollutionB".to_string(),
            _ => {}
        }
    }
    format!("pollution{index:02}")
}

fn generated_goop_requires_upgrade(authoring: &GoopAuthoringDocument) -> bool {
    authoring.requires_generator_upgrade()
}

impl SmsEditorApp {
    pub(super) fn ensure_goop_templates_indexed(&mut self) {
        if self.goop_templates_indexed {
            return;
        }
        let (templates, warnings) = index_retail_goop_templates(&self.scene_archives);
        self.retail_goop_templates = templates;
        self.goop_templates_indexed = true;
        self.log.extend(warnings);
        if self.selected_goop_template >= self.retail_goop_templates.len() {
            self.selected_goop_template = 0;
        }
    }

    pub(super) fn goop_inspector_panel(&mut self, ui: &mut egui::Ui) {
        self.ensure_goop_templates_indexed();
        let generator_upgrade_available =
            self.background_receiver.is_none() && self.goop_stroke.is_none();
        let Some(document) = self.document.as_mut() else {
            ui.label("Open a stage to author goop.");
            return;
        };
        let generator_upgrade_pending = match document.ensure_goop_authoring() {
            Ok(authoring) => {
                let pending = generated_goop_requires_upgrade(authoring);
                if pending && generator_upgrade_available {
                    // Claim the migration before scheduling it so a failed
                    // background rebuild falls back to the visible stale-layer
                    // action instead of retrying every frame.
                    authoring.format_version = GOOP_AUTHORING_FORMAT_VERSION;
                }
                pending
            }
            Err(error) => {
                ui.colored_label(egui::Color32::RED, error.to_string());
                return;
            }
        };
        if generator_upgrade_pending && generator_upgrade_available {
            self.rebuild_generated_goop_layers();
        }
        if self.background_label.as_deref() == Some("Rebuilding goopmaps") {
            ui.heading("Goop");
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Rebuilding generated goop for this editor version...");
            });
            ui.small(
                "The painted mask is preserved while the mesh and depth resources are repaired.",
            );
            return;
        }
        let document = self
            .document
            .as_mut()
            .expect("stage document remains open while drawing goop inspector");
        ui.heading("Goop");
        ui.small("Floor layers are editable. Retail wall and wave layers remain read-only.");
        ui.separator();

        let layers = document
            .goop_authoring
            .as_ref()
            .map(|goop| goop.layers.clone())
            .unwrap_or_default();
        for (index, layer) in layers.iter().enumerate() {
            let mut label = format!("{:02}  {}", layer.runtime_index, layer.behavior.label());
            if !layer.editable() {
                label.push_str("  (read-only)");
            }
            if ui
                .selectable_label(self.selected_goop_layer == index, label)
                .clicked()
            {
                self.selected_goop_layer = index;
            }
        }
        self.selected_goop_layer = self.selected_goop_layer.min(layers.len().saturating_sub(1));

        let selected_generated_layer = layers
            .get(self.selected_goop_layer)
            .filter(|layer| layer.origin == GoopLayerOrigin::Generated);
        if let Some(source) = selected_generated_layer.and_then(|layer| layer.style_source.as_ref())
        {
            if let Some((index, _)) =
                self.retail_goop_templates
                    .iter()
                    .enumerate()
                    .find(|(_, template)| {
                        template.stage_id == source.stage_id
                            && template.layer_index == source.layer_index
                    })
            {
                self.selected_goop_template = index;
            }
        }

        ui.separator();
        ui.checkbox(
            &mut self.show_incompatible_goop_templates,
            "Show incompatible templates (expert)",
        );
        let selected_behavior = selected_generated_layer.map(|layer| layer.behavior);
        let visible_templates = self
            .retail_goop_templates
            .iter()
            .enumerate()
            .filter(|(_, template)| {
                (template.compatible
                    && selected_behavior.is_none_or(|behavior| template.behavior == behavior))
                    || self.show_incompatible_goop_templates
            })
            .map(|(index, template)| (index, template.clone()))
            .collect::<Vec<_>>();
        let selected_text = self
            .retail_goop_templates
            .get(self.selected_goop_template)
            .map_or("No compatible retail template".to_string(), |template| {
                format!(
                    "{} / {} ({})",
                    template.stage_id,
                    template.resource_stem,
                    template.behavior.label()
                )
            });
        let previous_template = self.selected_goop_template;
        egui::ComboBox::from_label(if selected_generated_layer.is_some() {
            "Retail style"
        } else {
            "New layer style"
        })
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            for (index, template) in visible_templates {
                let suffix = if template.compatible {
                    ""
                } else {
                    " [incompatible]"
                };
                ui.selectable_value(
                    &mut self.selected_goop_template,
                    index,
                    format!(
                        "{} / {}{}",
                        template.stage_id, template.resource_stem, suffix
                    ),
                );
            }
        });
        if self.selected_goop_template != previous_template && selected_generated_layer.is_some() {
            self.set_selected_goop_style(self.selected_goop_template);
            return;
        }
        if self
            .retail_goop_templates
            .get(self.selected_goop_template)
            .is_some_and(|template| {
                !template.compatible
                    || selected_behavior.is_some_and(|behavior| template.behavior != behavior)
            })
        {
            ui.colored_label(
                egui::Color32::from_rgb(245, 180, 70),
                "Expert override: this template is structurally or behavior-incompatible.",
            );
        }
        ui.checkbox(&mut self.goop_use_custom_region, "Use manual region");
        if self.goop_use_custom_region {
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.goop_region_min_x)
                        .speed(GOOP_CELL_SIZE)
                        .prefix("Min X "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.goop_region_min_z)
                        .speed(GOOP_CELL_SIZE)
                        .prefix("Min Z "),
                );
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.goop_region_width_cells)
                        .range(8..=1024)
                        .prefix("Cells X "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.goop_region_height_cells)
                        .range(4..=1024)
                        .prefix("Cells Z "),
                );
            });
            ui.small("Origins must align to 40 units; cell dimensions must be powers of two.");
        }
        if ui
            .add_enabled(
                layers.len() < GOOP_MAX_LAYERS
                    && self
                        .retail_goop_templates
                        .get(self.selected_goop_template)
                        .is_some(),
                egui::Button::new("Add generated floor layer"),
            )
            .clicked()
        {
            self.add_generated_goop_layer();
            return;
        }

        ui.separator();
        ui.label("Brush");
        ui.add(egui::Slider::new(&mut self.goop_brush_radius, 20.0..=2000.0).text("Radius"));
        ui.add(egui::Slider::new(&mut self.goop_brush_hardness, 0.0..=1.0).text("Hardness"));
        ui.add(egui::Slider::new(&mut self.goop_brush_opacity, 0.01..=1.0).text("Opacity"));
        ui.checkbox(&mut self.goop_fill_mode, "Connected fill");
        ui.small("Left-drag paints. Shift + left-drag erases.");

        let selected = self.document.as_ref().and_then(|document| {
            document
                .goop_authoring
                .as_ref()
                .and_then(|goop| goop.layers.get(self.selected_goop_layer))
                .cloned()
        });
        if let Some(layer) = selected {
            ui.separator();
            if layer.editable() {
                ui.colored_label(
                    egui::Color32::from_rgb(100, 220, 140),
                    "Brush ready: hold left mouse over a valid floor cell.",
                );
            } else {
                ui.colored_label(
                    egui::Color32::RED,
                    "This layer has no editable pollution BMP mask.",
                );
            }
            let mut visible = layer.visible;
            if ui
                .checkbox(&mut visible, "Visible in authoring overlay")
                .changed()
            {
                self.set_selected_goop_visibility(visible);
            }
            let mut behavior = layer.behavior;
            egui::ComboBox::from_label("Runtime behavior")
                .selected_text(behavior.label())
                .show_ui(ui, |ui| {
                    for preset in [
                        GoopBehavior::Normal,
                        GoopBehavior::Fire,
                        GoopBehavior::Slippery,
                        GoopBehavior::Barrier,
                        GoopBehavior::Electric,
                    ] {
                        ui.selectable_value(&mut behavior, preset, preset.label());
                    }
                    if matches!(layer.behavior, GoopBehavior::Retail(_)) {
                        ui.selectable_value(&mut behavior, layer.behavior, layer.behavior.label());
                    }
                });
            if behavior != layer.behavior {
                self.set_selected_goop_behavior(behavior);
            }
            let (width, height) = layer.dimensions().unwrap_or((0, 0));
            let coverage = layer
                .mask()
                .map(|mask| mask.iter().filter(|value| **value != 0).count())
                .unwrap_or(0);
            let valid = (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .filter(|(x, y)| layer.valid_cell(*x, *y))
                .count();
            ui.label(format!("Resolution: {width} x {height}"));
            ui.label(format!("Painted: {coverage} / {valid} valid cells"));
            ui.label(format!(
                "Region: X {:.0}..{:.0}, Z {:.0}..{:.0}",
                layer.region.min_x, layer.region.max_x, layer.region.min_z, layer.region.max_z
            ));
            ui.label(format!("Plane: {:?}", layer.plane));
            if let Some(style) = &layer.style_source {
                ui.small(format!("Style: {}", style.display_name));
                if style.forced_incompatible || style.behavior_code != layer.behavior.runtime_code()
                {
                    ui.colored_label(
                        egui::Color32::from_rgb(245, 180, 70),
                        "Persistent warning: behavior/style compatibility was overridden.",
                    );
                }
            }
            if layer.origin == GoopLayerOrigin::Generated
                && self.selected_goop_layer + 1 == layers.len()
                && ui.button("Delete generated layer").clicked()
            {
                self.delete_selected_goop_layer();
                return;
            }
        }
        if self
            .document
            .as_ref()
            .and_then(|document| document.goop_authoring.as_ref())
            .is_some_and(|goop| goop.stale)
        {
            ui.colored_label(
                egui::Color32::RED,
                "Generated goop resources need rebuilding because the terrain or generator changed.",
            );
            if ui.button("Rebuild generated layers").clicked() {
                self.rebuild_generated_goop_layers();
            }
        }
    }

    pub(super) fn add_generated_goop_layer(&mut self) {
        let Some(template) = self
            .retail_goop_templates
            .get(self.selected_goop_template)
            .cloned()
        else {
            self.log
                .push("No usable retail goop template is selected.".to_string());
            return;
        };
        let terrain = match self.final_goop_terrain_snapshot() {
            Ok(terrain) => terrain,
            Err(error) => {
                self.log.push(format!(
                    "Could not snapshot final terrain for goop generation: {error}"
                ));
                return;
            }
        };
        let custom_region = (
            self.goop_use_custom_region,
            self.goop_region_min_x,
            self.goop_region_min_z,
            self.goop_region_width_cells,
            self.goop_region_height_cells,
        );
        let Some(document) = &mut self.document else {
            return;
        };
        let before = document.goop_authoring.clone();
        let before_edits = document.archive_edits.clone();
        let result = (|| -> Result<(), String> {
            let target_stage_id = document.stage_id.clone();
            let (use_custom, min_x, min_z, width_cells, height_cells) = custom_region;
            let (region, width_log2, height_log2) = if use_custom {
                if (min_x / GOOP_CELL_SIZE).fract().abs() > 0.0001
                    || (min_z / GOOP_CELL_SIZE).fract().abs() > 0.0001
                {
                    return Err(
                        "Manual goop region origins must align to 40-unit cells".to_string()
                    );
                }
                if !width_cells.is_power_of_two()
                    || !height_cells.is_power_of_two()
                    || width_cells < 8
                    || height_cells < 4
                {
                    return Err(
                        "Manual goop region dimensions must be power-of-two cells (minimum 8x4)"
                            .to_string(),
                    );
                }
                let region = sms_scene::GoopRegion {
                    min_x,
                    min_z,
                    max_x: min_x + f32::from(width_cells) * GOOP_CELL_SIZE,
                    max_z: min_z + f32::from(height_cells) * GOOP_CELL_SIZE,
                };
                (
                    region,
                    width_cells.trailing_zeros() as u16,
                    height_cells.trailing_zeros() as u16,
                )
            } else {
                whole_terrain_region(&terrain.collision_triangles)
                    .map_err(|error| error.to_string())?
            };
            let (vertical_offset, depth_map) = generate_floor_depth_map(
                &terrain.collision_triangles,
                region,
                width_log2,
                height_log2,
            )
            .map_err(|error| error.to_string())?;
            let width = 1u16 << width_log2;
            let height = 1u16 << height_log2;
            let template_model = read_stage_asset_bytes(&template.model_asset_path)
                .and_then(J3dRebuildDocument::parse)
                .map_err(|error| error.to_string())?;
            let generated_model = generate_floor_pollution_model(
                &template_model,
                &terrain.render_triangles,
                region,
                width,
                height,
                !template.compatible,
            )
            .map_err(|error| error.to_string())?;
            let authoring = document
                .ensure_goop_authoring()
                .map_err(|error| error.to_string())?;
            if authoring.layers.len() >= GOOP_MAX_LAYERS {
                return Err(format!(
                    "Sunshine supports at most {GOOP_MAX_LAYERS} goop layers"
                ));
            }
            if authoring
                .layers
                .iter()
                .any(|layer| layer.plane == GoopPlane::Floor && layer.region.overlaps(region))
            {
                return Err(
                    "The default whole-terrain region overlaps an existing floor layer. Edit or subdivide regions before adding another layer."
                        .to_string(),
                );
            }
            let runtime_index = authoring.layers.len();
            let target_stem = source_pollution_stem(runtime_index, &target_stage_id);
            authoring.layers.push(GoopLayerAuthoring {
                id: format!("generated-goop-{runtime_index:02}"),
                runtime_index,
                origin: GoopLayerOrigin::Generated,
                plane: GoopPlane::Floor,
                behavior: template.behavior,
                visible: true,
                region,
                runtime: YmpLayer {
                    layer_type: template.behavior.runtime_code(),
                    subtype: 0,
                    flags: GoopPlane::Floor.runtime_code(),
                    reserved: 0,
                    vertical_offset,
                    vertical_scale: GOOP_CELL_SIZE,
                    min_x: region.min_x,
                    min_z: region.min_z,
                    max_x: region.max_x,
                    max_z: region.max_z,
                    width_log2,
                    height_log2,
                    user_value: 0,
                    map_offset: 0,
                    depth_map,
                },
                bitmap: Some(
                    BmpFile::new_pollution_mask(
                        width,
                        height,
                        vec![0; usize::from(width) * usize::from(height)],
                    )
                    .map_err(|error| error.to_string())?,
                ),
                generated_model: Some(generated_model),
                style_source: Some(GoopStyleSource {
                    stage_id: template.stage_id.clone(),
                    layer_index: template.layer_index,
                    display_name: format!("{} / {}", template.stage_id, template.resource_stem),
                    behavior_code: template.behavior.runtime_code(),
                    forced_incompatible: !template.compatible,
                }),
                resource_stem: target_stem.clone(),
                metadata_dirty: true,
            });
            authoring.terrain_fingerprint = terrain.fingerprint;
            authoring.stale = false;
            document
                .compile_goop_authoring()
                .map_err(|error| error.to_string())?;
            copy_template_animations(document, &template, &target_stem)?;
            Ok(())
        })();
        if let Err(error) = result {
            document.goop_authoring = before;
            document.archive_edits = before_edits;
            self.log
                .push(format!("Could not generate goop layer: {error}"));
            return;
        }
        let record = GoopUndoRecord::Snapshot(Box::new(GoopSnapshotUndo {
            before,
            after: document.goop_authoring.clone(),
            before_edits,
            after_edits: document.archive_edits.clone(),
        }));
        self.selected_goop_layer = document
            .goop_authoring
            .as_ref()
            .map_or(0, |goop| goop.layers.len().saturating_sub(1));
        self.push_goop_undo(record);
        self.finish_goop_document_change("Generated playable floor goop layer");
    }

    fn finalized_goop_terrain_document(&self) -> Result<StageDocument, String> {
        let mut document = self
            .document
            .clone()
            .ok_or_else(|| "no stage is open".to_string())?;
        let instances = self
            .model_instances
            .iter()
            .filter(|instance| instance.stage_id.eq_ignore_ascii_case(&document.stage_id))
            .cloned()
            .collect::<Vec<_>>();
        let assets = if instances.is_empty() {
            BTreeMap::new()
        } else {
            let content_root = self
                .model_content_root()
                .ok_or_else(|| "project Content root is unavailable".to_string())?;
            Self::load_model_asset_snapshot(&content_root, &instances)?
        };
        let not_cancelled = AtomicBool::new(false);
        document.archive_edits = Self::stage_edits_with_model_instances_from_snapshot_cancellable(
            &assets,
            &instances,
            &document.archive_edits,
            document.stage_archive.as_ref(),
            document.registry.as_ref(),
            &not_cancelled,
        )?;
        Ok(document)
    }

    fn final_goop_terrain_snapshot(&self) -> Result<FinalGoopTerrainSnapshot, String> {
        let document = self.finalized_goop_terrain_document()?;
        let fingerprint = document
            .effective_terrain_fingerprint()
            .map_err(|error| error.to_string())?;

        let collision = match document
            .effective_resource_clone(b"map/map.col")
            .map_err(|error| error.to_string())?
        {
            Some(resource) => Some(resource),
            None => document
                .effective_resource_clone(b"map/map/map.col")
                .map_err(|error| error.to_string())?,
        };
        let collision = match collision {
            Some(StageResourceDocument::Collision(collision)) => collision,
            Some(_) => {
                return Err("effective map collision resource is not COL data".to_string());
            }
            None => {
                return Err(
                    "final terrain has no map/map.col or map/map/map.col resource".to_string(),
                );
            }
        };
        let collision_triangles = goop_collision_triangles(&collision);
        if collision_triangles.is_empty() {
            return Err("final map collision contains no triangles".to_string());
        }

        let model = match document
            .effective_resource_clone(b"map/map/map.bmd")
            .map_err(|error| error.to_string())?
        {
            Some(StageResourceDocument::Model(model)) => model,
            Some(_) => return Err("effective map/map/map.bmd is not model data".to_string()),
            None => return Err("final terrain has no map/map/map.bmd resource".to_string()),
        };
        let model_bytes = model.to_bytes().map_err(|error| error.to_string())?;
        let preview = J3dFile::parse(&model_bytes)
            .and_then(|model| model.geometry_preview_with_loader_flags(SMS_MAP_MODEL_LOAD_FLAGS))
            .map_err(|error| format!("could not decode final map render geometry: {error}"))?;
        let render_triangles = goop_upward_render_triangles(&preview, &collision_triangles);
        if render_triangles.is_empty() {
            return Err("final map model contains no upward-facing render triangles".to_string());
        }

        Ok(FinalGoopTerrainSnapshot {
            collision_triangles,
            render_triangles,
            fingerprint,
        })
    }

    pub(super) fn final_goop_terrain_fingerprint(&self) -> Result<u64, String> {
        let document = self.finalized_goop_terrain_document()?;
        document
            .effective_terrain_fingerprint()
            .map_err(|error| error.to_string())
    }

    pub(super) fn refresh_goop_stale_from_final_terrain(&mut self) {
        let should_check = self.document.as_ref().is_some_and(|document| {
            document.goop_authoring.as_ref().is_some_and(|goop| {
                goop.layers
                    .iter()
                    .any(|layer| layer.origin == GoopLayerOrigin::Generated)
                    && goop.terrain_fingerprint != 0
            })
        });
        if !should_check {
            return;
        }
        let Ok(fingerprint) = self.final_goop_terrain_fingerprint() else {
            return;
        };
        if let Some(goop) = self
            .document
            .as_mut()
            .and_then(|document| document.goop_authoring.as_mut())
        {
            goop.stale |= goop.terrain_fingerprint != fingerprint;
        }
    }

    pub(super) fn handle_goop_viewport_input(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        if self.tool != EditorTool::Goop || ui.input(|input| input.modifiers.alt) {
            return false;
        }
        if self.background_label.as_deref() == Some("Rebuilding goopmaps") {
            self.goop_cursor_world = None;
            return false;
        }
        let pointer = ui.input(|input| input.pointer.interact_pos());
        self.goop_cursor_world = pointer.and_then(|position| self.goop_surface_hit(rect, position));
        if !response.hovered() && self.goop_stroke.is_none() {
            return false;
        }
        let down = ui.input(|input| input.pointer.primary_down());
        let released = ui.input(|input| input.pointer.primary_released());
        let had_stroke = self.goop_stroke.is_some();
        if down && self.goop_stroke.is_none() && self.goop_cursor_world.is_some() {
            if self
                .document
                .as_mut()
                .and_then(|document| document.ensure_goop_authoring().ok())
                .is_some_and(|goop| goop.layers.is_empty())
            {
                self.ensure_goop_templates_indexed();
                self.add_generated_goop_layer();
            }
            let editable = self.document.as_ref().is_some_and(|document| {
                document
                    .goop_authoring
                    .as_ref()
                    .and_then(|goop| goop.layers.get(self.selected_goop_layer))
                    .is_some_and(GoopLayerAuthoring::editable)
            });
            if editable {
                self.goop_stroke = Some(GoopStroke {
                    layer: self.selected_goop_layer,
                    changed: BTreeMap::new(),
                    last_world: None,
                });
            }
        }
        if down {
            if let Some(world) = self.goop_cursor_world {
                let erase = ui.input(|input| input.modifiers.shift);
                if self.paint_goop_toward(world, erase) {
                    self.refresh_live_goop_preview();
                }
                ui.ctx().request_repaint();
            }
        }
        if released && self.goop_stroke.is_some() {
            self.commit_goop_stroke();
        }
        self.goop_stroke.is_some() || had_stroke || down
    }

    fn paint_goop_toward(&mut self, world: [f32; 3], erase: bool) -> bool {
        let Some(stroke) = self.goop_stroke.as_ref() else {
            return false;
        };
        let layer_index = stroke.layer;
        let cell_size = self
            .document
            .as_ref()
            .and_then(|document| document.goop_authoring.as_ref())
            .and_then(|goop| goop.layers.get(layer_index))
            .map_or(GOOP_CELL_SIZE, |layer| layer.runtime.vertical_scale);
        let samples = stroke.last_world.map_or_else(
            || vec![world],
            |last| {
                let dx = world[0] - last[0];
                let dz = world[2] - last[2];
                let distance = dx.hypot(dz);
                let spacing = (self.goop_brush_radius * 0.25).max(cell_size * 0.25);
                let count = (distance / spacing).ceil().max(1.0) as usize;
                (1..=count)
                    .map(|step| {
                        let t = step as f32 / count as f32;
                        [last[0] + dx * t, world[1], last[2] + dz * t]
                    })
                    .collect()
            },
        );
        let mut all_changes = Vec::new();
        let Some(document) = &mut self.document else {
            return false;
        };
        let Some(layer) = document
            .goop_authoring
            .as_mut()
            .and_then(|goop| goop.layers.get_mut(layer_index))
        else {
            return false;
        };
        let Ok(mut mask) = layer.mask() else {
            return false;
        };
        if self.goop_fill_mode {
            if let Some((x, y)) = layer.world_to_cell(world[0], world[2]) {
                flood_fill(layer, &mut mask, [x, y], erase, &mut all_changes);
            }
        } else {
            for sample in samples {
                paint_brush_sample(
                    layer,
                    &mut mask,
                    sample,
                    GoopBrush {
                        radius: self.goop_brush_radius,
                        hardness: self.goop_brush_hardness,
                        opacity: self.goop_brush_opacity,
                        erase,
                    },
                    &mut all_changes,
                );
            }
        }
        if layer.set_mask(&mask).is_err() {
            return false;
        }
        let changed = !all_changes.is_empty();
        if let Some(stroke) = &mut self.goop_stroke {
            for (index, before, after) in all_changes {
                stroke
                    .changed
                    .entry(index)
                    .and_modify(|change| change.1 = after)
                    .or_insert((before, after));
            }
            stroke.last_world = Some(world);
        }
        changed
    }

    fn refresh_live_goop_preview(&mut self) {
        let Some((width, height, mask)) = self.document.as_ref().and_then(|document| {
            let layer = document
                .goop_authoring
                .as_ref()?
                .layers
                .get(self.selected_goop_layer)?;
            let (width, height) = layer.dimensions().ok()?;
            Some((width, height, layer.mask().ok()?))
        }) else {
            return;
        };
        let Some(texture_indices) = self
            .model_preview
            .as_ref()
            .and_then(|preview| {
                preview
                    .pollution_texture_indices
                    .get(&self.selected_goop_layer)
            })
            .cloned()
        else {
            return;
        };
        let mut rgba = Vec::with_capacity(mask.len() * 4);
        for value in mask {
            rgba.extend_from_slice(&[value, value, value, value]);
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
        let Some(preview) = self.model_preview.as_mut() else {
            return;
        };
        for index in texture_indices.iter().copied() {
            let Some(texture) = preview.textures.get_mut(index) else {
                continue;
            };
            if texture.image.size != [width, height] {
                continue;
            }
            texture.image = image.clone();
            texture.mips.clear();
            texture.mips.push(image.clone());
            texture.mipmap_enabled = false;
            texture.mipmap_count = 1;
            texture.has_alpha = true;
            texture.has_translucent_alpha = rgba
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 12 && pixel[3] < 245);
        }
        if let Some(gpu_viewport) = &self.gpu_viewport {
            gpu_viewport.update_textures(preview, &texture_indices);
        }
        self.clear_viewport_preview_cache();
    }

    fn commit_goop_stroke(&mut self) {
        let Some(stroke) = self.goop_stroke.take() else {
            return;
        };
        if stroke.changed.is_empty() {
            return;
        }
        let spans = coalesce_pixel_changes(stroke.changed);
        if let Some(document) = &mut self.document {
            if let Err(error) = document.compile_goop_authoring() {
                self.log
                    .push(format!("Could not compile painted goop mask: {error}"));
                return;
            }
        }
        self.push_goop_undo(GoopUndoRecord::Pixels {
            layer: stroke.layer,
            spans,
        });
        self.finish_goop_document_change("Painted goop mask");
    }

    fn push_goop_undo(&mut self, record: GoopUndoRecord) {
        self.goop_undo_stack.push_back(record);
        if self.goop_undo_stack.len() > 80 {
            self.goop_undo_stack.pop_front();
        }
        self.goop_redo_stack.clear();
    }

    fn set_selected_goop_visibility(&mut self, visible: bool) {
        let Some(document) = &mut self.document else {
            return;
        };
        let before = document.goop_authoring.clone();
        let before_edits = document.archive_edits.clone();
        let Some(layer) = document
            .goop_authoring
            .as_mut()
            .and_then(|goop| goop.layers.get_mut(self.selected_goop_layer))
        else {
            return;
        };
        layer.visible = visible;
        let record = GoopUndoRecord::Snapshot(Box::new(GoopSnapshotUndo {
            before,
            after: document.goop_authoring.clone(),
            before_edits: before_edits.clone(),
            after_edits: document.archive_edits.clone(),
        }));
        self.push_goop_undo(record);
        self.finish_goop_document_change("Changed goop layer visibility");
    }

    fn set_selected_goop_behavior(&mut self, behavior: GoopBehavior) {
        let Some(document) = &mut self.document else {
            return;
        };
        let before = document.goop_authoring.clone();
        let before_edits = document.archive_edits.clone();
        let Some(layer) = document
            .goop_authoring
            .as_mut()
            .and_then(|goop| goop.layers.get_mut(self.selected_goop_layer))
        else {
            return;
        };
        layer.behavior = behavior;
        layer.runtime.layer_type = behavior.runtime_code();
        layer.metadata_dirty = true;
        if let Err(error) = document.compile_goop_authoring() {
            document.goop_authoring = before;
            document.archive_edits = before_edits;
            self.log
                .push(format!("Could not change goop behavior: {error}"));
            return;
        }
        let record = GoopUndoRecord::Snapshot(Box::new(GoopSnapshotUndo {
            before,
            after: document.goop_authoring.clone(),
            before_edits,
            after_edits: document.archive_edits.clone(),
        }));
        self.push_goop_undo(record);
        self.finish_goop_document_change("Changed goop runtime behavior");
    }

    fn set_selected_goop_style(&mut self, template_index: usize) {
        let Some(template) = self.retail_goop_templates.get(template_index).cloned() else {
            return;
        };
        let terrain = match self.final_goop_terrain_snapshot() {
            Ok(terrain) => terrain,
            Err(error) => {
                self.log.push(format!(
                    "Could not snapshot final terrain for the goop style change: {error}"
                ));
                return;
            }
        };
        let template_model = match read_stage_asset_bytes(&template.model_asset_path)
            .and_then(J3dRebuildDocument::parse)
        {
            Ok(model) => model,
            Err(error) => {
                self.log.push(format!(
                    "Could not read the selected retail goop style: {error}"
                ));
                return;
            }
        };
        let Some((region, width, height, behavior, target_stem)) = self
            .document
            .as_ref()
            .and_then(|document| document.goop_authoring.as_ref())
            .and_then(|goop| goop.layers.get(self.selected_goop_layer))
            .filter(|layer| layer.origin == GoopLayerOrigin::Generated)
            .and_then(|layer| {
                layer.dimensions().ok().map(|(width, height)| {
                    (
                        layer.region,
                        width,
                        height,
                        layer.behavior,
                        layer.resource_stem.clone(),
                    )
                })
            })
        else {
            return;
        };
        let generated_model = match generate_floor_pollution_model(
            &template_model,
            &terrain.render_triangles,
            region,
            width as u16,
            height as u16,
            !template.compatible,
        ) {
            Ok(model) => model,
            Err(error) => {
                self.log
                    .push(format!("Could not apply the selected goop style: {error}"));
                return;
            }
        };

        let Some(document) = &mut self.document else {
            return;
        };
        let before = document.goop_authoring.clone();
        let before_edits = document.archive_edits.clone();
        let result = (|| -> Result<(), String> {
            let layer = document
                .goop_authoring
                .as_mut()
                .and_then(|goop| goop.layers.get_mut(self.selected_goop_layer))
                .ok_or_else(|| "the selected goop layer no longer exists".to_string())?;
            layer.generated_model = Some(generated_model);
            layer.style_source = Some(GoopStyleSource {
                stage_id: template.stage_id.clone(),
                layer_index: template.layer_index,
                display_name: format!("{} / {}", template.stage_id, template.resource_stem),
                behavior_code: template.behavior.runtime_code(),
                forced_incompatible: !template.compatible || template.behavior != behavior,
            });
            layer.metadata_dirty = true;
            for extension in ["btk", "btp", "brk", "bck", "bas"] {
                document.archive_edits.remove_resource(
                    format!("map/pollution/{target_stem}.{extension}").into_bytes(),
                );
            }
            document
                .compile_goop_authoring()
                .map_err(|error| error.to_string())?;
            copy_template_animations(document, &template, &target_stem)
        })();
        if let Err(error) = result {
            document.goop_authoring = before;
            document.archive_edits = before_edits;
            self.log
                .push(format!("Could not change the goop retail style: {error}"));
            return;
        }
        let record = GoopUndoRecord::Snapshot(Box::new(GoopSnapshotUndo {
            before,
            after: document.goop_authoring.clone(),
            before_edits,
            after_edits: document.archive_edits.clone(),
        }));
        self.push_goop_undo(record);
        self.finish_goop_document_change("Changed goop retail style");
    }

    pub(super) fn undo_goop(&mut self) -> bool {
        let Some(record) = self.goop_undo_stack.pop_back() else {
            return false;
        };
        self.apply_goop_undo_record(&record, false);
        self.goop_redo_stack.push_back(record);
        self.finish_goop_document_change("Undo goop edit");
        true
    }

    pub(super) fn redo_goop(&mut self) -> bool {
        let Some(record) = self.goop_redo_stack.pop_back() else {
            return false;
        };
        self.apply_goop_undo_record(&record, true);
        self.goop_undo_stack.push_back(record);
        self.finish_goop_document_change("Redo goop edit");
        true
    }

    fn apply_goop_undo_record(&mut self, record: &GoopUndoRecord, forward: bool) {
        let Some(document) = &mut self.document else {
            return;
        };
        match record {
            GoopUndoRecord::Pixels { layer, spans } => {
                let Some(target) = document
                    .goop_authoring
                    .as_mut()
                    .and_then(|goop| goop.layers.get_mut(*layer))
                else {
                    return;
                };
                let Ok(mut mask) = target.mask() else { return };
                for span in spans {
                    let values = if forward { &span.after } else { &span.before };
                    let end = span.start + values.len();
                    if let Some(destination) = mask.get_mut(span.start..end) {
                        destination.copy_from_slice(values);
                    }
                }
                let _ = target.set_mask(&mask);
                let _ = document.compile_goop_authoring();
            }
            GoopUndoRecord::Snapshot(snapshot) => {
                document.goop_authoring = if forward {
                    snapshot.after.clone()
                } else {
                    snapshot.before.clone()
                };
                document.archive_edits = if forward {
                    snapshot.after_edits.clone()
                } else {
                    snapshot.before_edits.clone()
                };
            }
        }
    }

    fn finish_goop_document_change(&mut self, label: &str) {
        self.document_dirty = true;
        self.flush_document_change();
        self.rebuild_model_preview_from_document();
        self.log.push(format!("{label}."));
    }

    fn delete_selected_goop_layer(&mut self) {
        let Some(document) = &mut self.document else {
            return;
        };
        let before = document.goop_authoring.clone();
        let before_edits = document.archive_edits.clone();
        let Some(authoring) = &mut document.goop_authoring else {
            return;
        };
        if self.selected_goop_layer + 1 != authoring.layers.len()
            || authoring.layers[self.selected_goop_layer].origin != GoopLayerOrigin::Generated
        {
            self.log
                .push("Only the last generated layer can be deleted safely.".to_string());
            return;
        }
        let layer = authoring
            .layers
            .pop()
            .expect("selected generated layer exists");
        let mut export_authoring = authoring.clone();
        export_authoring.stale = false;
        let compiled_ymp = match document.effective_resource_clone(sms_scene::GOOP_RESOURCE_PATH) {
            Ok(Some(StageResourceDocument::PollutionMap(base))) => {
                export_authoring.compiled_ymp_preserving(&base)
            }
            Ok(_) => export_authoring.compiled_ymp(),
            Err(error) => Err(error),
        };
        let compiled_ymp = match compiled_ymp {
            Ok(ymp) => ymp,
            Err(error) => {
                document.goop_authoring = before;
                document.archive_edits = before_edits;
                self.log
                    .push(format!("Could not delete goop layer: {error}"));
                return;
            }
        };
        let imported_ymp_exists = document
            .stage_archive
            .as_ref()
            .is_some_and(|archive| archive.resource(sms_scene::GOOP_RESOURCE_PATH).is_some());
        if export_authoring.layers.is_empty() && !imported_ymp_exists {
            document
                .archive_edits
                .remove_resource(sms_scene::GOOP_RESOURCE_PATH.to_vec());
        } else {
            document.upsert_authored_resource(
                sms_scene::GOOP_RESOURCE_PATH.to_vec(),
                StageResourceDocument::PollutionMap(compiled_ymp),
            );
        }
        for extension in ["bmp", "bmd", "btk", "btp", "brk", "bck", "bas"] {
            document.archive_edits.remove_resource(
                format!("map/pollution/{}.{extension}", layer.resource_stem).into_bytes(),
            );
        }
        if let Err(error) = document.compile_goop_authoring() {
            document.goop_authoring = before;
            document.archive_edits = before_edits;
            self.log
                .push(format!("Could not delete goop layer: {error}"));
            return;
        }
        let record = GoopUndoRecord::Snapshot(Box::new(GoopSnapshotUndo {
            before,
            after: document.goop_authoring.clone(),
            before_edits,
            after_edits: document.archive_edits.clone(),
        }));
        self.push_goop_undo(record);
        self.selected_goop_layer = self.selected_goop_layer.saturating_sub(1);
        self.finish_goop_document_change("Deleted generated goop layer");
    }

    fn rebuild_generated_goop_layers(&mut self) {
        if self.background_receiver.is_some() {
            self.log
                .push("Another background operation is already running.".to_string());
            return;
        }
        let terrain = match self.final_goop_terrain_snapshot() {
            Ok(terrain) => terrain,
            Err(error) => {
                self.log.push(format!(
                    "Could not snapshot final terrain for goop rebuild: {error}"
                ));
                return;
            }
        };
        let templates = self.retail_goop_templates.clone();
        let Some(document) = self.document.clone() else {
            return;
        };
        let before = document.goop_authoring.clone();
        let before_edits = document.archive_edits.clone();
        let base_root = self.base_root.trim().to_string();
        let stage_id = document.stage_id.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let task_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut document = document;
            let result = rebuild_goop_document(
                &mut document,
                &terrain.collision_triangles,
                &terrain.render_triangles,
                &templates,
                terrain.fingerprint,
                &task_cancel,
            )
            .map(|()| {
                Box::new(GoopRebuildOutcome {
                    base_root,
                    stage_id,
                    terrain_fingerprint: terrain.fingerprint,
                    before,
                    before_edits,
                    document,
                })
            });
            let _ = sender.send(BackgroundResult::GoopRebuild(result));
        });
        self.background_receiver = Some(receiver);
        self.active_build_cancel = Some(cancel);
        self.background_label = Some("Rebuilding goopmaps".to_string());
        self.log
            .push("Rebuilding stale goop layers from the current terrain snapshot...".to_string());
    }

    pub(super) fn apply_goop_rebuild(&mut self, outcome: GoopRebuildOutcome) {
        if self.base_root.trim() != outcome.base_root || self.stage_id != outcome.stage_id {
            self.log
                .push("Discarded rebuilt goopmaps because the open stage changed.".to_string());
            return;
        }
        let current_fingerprint = match self.final_goop_terrain_fingerprint() {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.log.push(format!(
                    "Discarded rebuilt goopmaps because the current terrain could not be verified: {error}"
                ));
                return;
            }
        };
        if current_fingerprint != outcome.terrain_fingerprint {
            self.log.push(
                "Discarded rebuilt goopmaps because the terrain or a map-terrain instance changed during the rebuild."
                    .to_string(),
            );
            return;
        }
        let Some(current) = &self.document else {
            return;
        };
        if current.goop_authoring != outcome.before || current.archive_edits != outcome.before_edits
        {
            self.log.push(
                "Discarded rebuilt goopmaps because the stage was edited during the rebuild."
                    .to_string(),
            );
            return;
        }
        let record = GoopUndoRecord::Snapshot(Box::new(GoopSnapshotUndo {
            before: outcome.before,
            after: outcome.document.goop_authoring.clone(),
            before_edits: outcome.before_edits,
            after_edits: outcome.document.archive_edits.clone(),
        }));
        self.document = Some(outcome.document);
        self.push_goop_undo(record);
        self.finish_goop_document_change("Rebuilt stale goop layers");
    }

    fn goop_surface_hit(&self, rect: egui::Rect, position: egui::Pos2) -> Option<[f32; 3]> {
        if !rect.contains(position) {
            return None;
        }
        let frame = self.camera_frame();
        let focal = perspective_focal_length(rect, self.viewport_zoom);
        let local = position - rect.center() - self.viewport_pan;
        let ray = vec3_normalize(vec3_add(
            frame.forward,
            vec3_add(
                vec3_scale(frame.right, local.x / focal),
                vec3_scale(frame.up, -local.y / focal),
            ),
        ));
        let preview = self.model_preview.as_ref()?;
        preview
            .collision_triangles
            .iter()
            .map(|triangle| triangle.vertices)
            .chain(
                preview
                    .triangles
                    .iter()
                    .filter(|triangle| triangle.render_layer == PreviewRenderLayer::Main)
                    .map(|triangle| triangle.vertices),
            )
            .filter_map(|vertices| {
                ray_triangle_distance(frame.position, ray, vertices).map(|distance| {
                    (
                        distance,
                        vec3_add(frame.position, vec3_scale(ray, distance)),
                    )
                })
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, hit)| hit)
    }

    pub(super) fn paint_goop_overlay(&self, painter: &egui::Painter, rect: egui::Rect) {
        if self.tool != EditorTool::Goop {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let Some(authoring) = &document.goop_authoring else {
            return;
        };
        let Some(layer) = authoring.layers.get(self.selected_goop_layer) else {
            return;
        };
        if !layer.visible {
            return;
        }
        let projection = self.camera_projection(rect);
        if let Ok((width, height)) = layer.dimensions() {
            let cell_size = layer.runtime.vertical_scale;
            let boundary_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 220, 150));
            for (cell_y, world_z) in [
                (0, layer.region.min_z),
                (height.saturating_sub(1), layer.region.max_z),
            ] {
                let points = (0..width).map(|cell_x| {
                    goop_cell_surface_y(layer, cell_x, cell_y).map(|world_y| {
                        [
                            layer.region.min_x + (cell_x as f32 + 0.5) * cell_size,
                            world_y + 8.0,
                            world_z,
                        ]
                    })
                });
                paint_surface_polyline(painter, projection, points, boundary_stroke);
            }
            for (cell_x, world_x) in [
                (0, layer.region.min_x),
                (width.saturating_sub(1), layer.region.max_x),
            ] {
                let points = (0..height).map(|cell_y| {
                    goop_cell_surface_y(layer, cell_x, cell_y).map(|world_y| {
                        [
                            world_x,
                            world_y + 8.0,
                            layer.region.min_z + (cell_y as f32 + 0.5) * cell_size,
                        ]
                    })
                });
                paint_surface_polyline(painter, projection, points, boundary_stroke);
            }

            let step = (((width * height) as f32 / 2048.0).sqrt().ceil() as usize).max(1);
            for cell_y in (0..height).step_by(step) {
                for cell_x in (0..width).step_by(step) {
                    if layer.valid_cell(cell_x, cell_y) {
                        continue;
                    }
                    let Some(world_y) = goop_invalid_marker_surface_y(layer, cell_x, cell_y) else {
                        continue;
                    };
                    let world = [
                        layer.region.min_x + (cell_x as f32 + 0.5) * cell_size,
                        world_y + 8.0,
                        layer.region.min_z + (cell_y as f32 + 0.5) * cell_size,
                    ];
                    if let Some((screen, _)) = projection.project_world_to_screen(world) {
                        painter.circle_filled(
                            screen,
                            1.5,
                            egui::Color32::from_rgba_unmultiplied(255, 70, 70, 170),
                        );
                    }
                }
            }
        }
        if let Some(hit) = self.goop_cursor_world {
            if let (Some((center, _)), Some((edge, _))) = (
                self.project_world_to_screen(rect, hit),
                self.project_world_to_screen(
                    rect,
                    [hit[0] + self.goop_brush_radius, hit[1], hit[2]],
                ),
            ) {
                let color = if layer
                    .world_to_cell(hit[0], hit[2])
                    .is_some_and(|(x, y)| layer.valid_cell(x, y))
                {
                    egui::Color32::from_rgb(255, 224, 100)
                } else {
                    egui::Color32::from_rgb(255, 90, 90)
                };
                painter.circle_stroke(
                    center,
                    center.distance(edge).max(2.0),
                    egui::Stroke::new(2.0, color),
                );
            }
        }
        if authoring.stale {
            painter.text(
                rect.center_top() + egui::vec2(0.0, 18.0),
                egui::Align2::CENTER_TOP,
                "GOOPMAP STALE — rebuild before release",
                egui::FontId::proportional(18.0),
                egui::Color32::RED,
            );
        }
    }
}

fn goop_cell_surface_y(layer: &GoopLayerAuthoring, x: usize, y: usize) -> Option<f32> {
    let depth = layer.runtime.depth_at(x, y).ok()?;
    (depth != 0xff)
        .then_some(layer.runtime.vertical_offset + f32::from(depth) * layer.runtime.vertical_scale)
}

fn goop_invalid_marker_surface_y(layer: &GoopLayerAuthoring, x: usize, y: usize) -> Option<f32> {
    let (width, height) = layer.dimensions().ok()?;
    let min_x = x.saturating_sub(1);
    let max_x = (x + 1).min(width.saturating_sub(1));
    let min_y = y.saturating_sub(1);
    let max_y = (y + 1).min(height.saturating_sub(1));
    (min_y..=max_y)
        .flat_map(|near_y| (min_x..=max_x).map(move |near_x| (near_x, near_y)))
        .filter_map(|(near_x, near_y)| goop_cell_surface_y(layer, near_x, near_y))
        .max_by(f32::total_cmp)
}

fn paint_surface_polyline(
    painter: &egui::Painter,
    projection: CameraProjection,
    points: impl IntoIterator<Item = Option<[f32; 3]>>,
    stroke: egui::Stroke,
) {
    let mut previous = None;
    for point in points {
        let projected = point.and_then(|point| projection.project_world_to_screen(point));
        if let (Some((from, _)), Some((to, _))) = (previous, projected) {
            painter.line_segment([from, to], stroke);
        }
        previous = projected;
    }
}

fn goop_collision_triangles(collision: &ColFile) -> Vec<GoopTerrainTriangle> {
    collision
        .groups()
        .iter()
        .flat_map(|group| &group.triangles)
        .filter_map(|triangle| {
            let [a, b, c] = triangle.vertex_indices;
            let vertices = [a, b, c].map(|index| {
                collision
                    .vertices()
                    .get(usize::from(index))
                    .map(|vertex| vertex.position)
            });
            let [Some(a), Some(b), Some(c)] = vertices else {
                return None;
            };
            Some(GoopTerrainTriangle {
                vertices: [a, b, c],
            })
        })
        .collect()
}

fn goop_upward_render_triangles(
    preview: &J3dGeometryPreview,
    collision_triangles: &[GoopTerrainTriangle],
) -> Vec<GoopRenderTriangle> {
    preview
        .triangles
        .iter()
        .filter(|triangle| {
            render_triangle_is_upward_facing(triangle.vertices, triangle.normals)
                && render_triangle_matches_topmost_collision(triangle.vertices, collision_triangles)
        })
        .map(|triangle| GoopRenderTriangle {
            vertices: triangle.vertices,
            normals: triangle.normals,
        })
        .collect()
}

fn render_triangle_is_upward_facing(
    vertices: [[f32; 3]; 3],
    normals: Option<[[f32; 3]; 3]>,
) -> bool {
    const MIN_UPWARD_COMPONENT: f32 = 0.1;

    if let Some(normals) = normals {
        let average = normals.iter().fold([0.0; 3], |mut average, normal| {
            for axis in 0..3 {
                average[axis] += normal[axis];
            }
            average
        });
        let length = vec3_dot(average, average).sqrt();
        if length.is_finite() && length > f32::EPSILON {
            return average[1] / length > MIN_UPWARD_COMPONENT;
        }
    }

    // GX runtime triangles use clockwise winding. In Sunshine coordinates an
    // upward-facing X/Z triangle therefore has a negative geometric Y normal.
    let edge_a = vec3_sub(vertices[1], vertices[0]);
    let edge_b = vec3_sub(vertices[2], vertices[0]);
    let geometric = vec3_cross(edge_a, edge_b);
    let length = vec3_dot(geometric, geometric).sqrt();
    length.is_finite() && length > f32::EPSILON && -geometric[1] / length > MIN_UPWARD_COMPONENT
}

fn render_triangle_matches_topmost_collision(
    vertices: [[f32; 3]; 3],
    collision_triangles: &[GoopTerrainTriangle],
) -> bool {
    const MAX_RENDER_COLLISION_SEPARATION: f32 = 30.0;

    let center: [f32; 3] =
        std::array::from_fn(|axis| vertices.iter().map(|vertex| vertex[axis]).sum::<f32>() / 3.0);
    collision_triangles
        .iter()
        .filter_map(|triangle| collision_height_at_xz(triangle.vertices, center[0], center[2]))
        .max_by(f32::total_cmp)
        .is_some_and(|height| (height - center[1]).abs() <= MAX_RENDER_COLLISION_SEPARATION)
}

fn collision_height_at_xz(vertices: [[f32; 3]; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = vertices;
    let denominator = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if denominator.abs() <= f32::EPSILON {
        return None;
    }
    let weight_a = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / denominator;
    let weight_b = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / denominator;
    let weight_c = 1.0 - weight_a - weight_b;
    (weight_a >= -0.0001 && weight_b >= -0.0001 && weight_c >= -0.0001)
        .then_some(weight_a * a[1] + weight_b * b[1] + weight_c * c[1])
}

fn rebuild_goop_document(
    document: &mut StageDocument,
    collision_triangles: &[GoopTerrainTriangle],
    render_triangles: &[GoopRenderTriangle],
    templates: &[RetailGoopTemplate],
    terrain_fingerprint: u64,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let Some(authoring) = &mut document.goop_authoring else {
        return Ok(());
    };
    for layer in authoring
        .layers
        .iter_mut()
        .filter(|layer| layer.origin == GoopLayerOrigin::Generated)
    {
        if cancelled.load(Ordering::Acquire) {
            return Err("goop rebuild cancelled".to_string());
        }
        let old_mask = layer.mask().map_err(|error| error.to_string())?;
        let (width, height) = layer.dimensions().map_err(|error| error.to_string())?;
        let (offset, depth) = generate_floor_depth_map(
            collision_triangles,
            layer.region,
            layer.runtime.width_log2,
            layer.runtime.height_log2,
        )
        .map_err(|error| error.to_string())?;
        layer.runtime.vertical_offset = offset;
        layer.runtime.depth_map = depth;
        let mut reprojected = old_mask;
        for y in 0..height {
            for x in 0..width {
                if !layer.valid_cell(x, y) {
                    reprojected[y * width + x] = 0;
                }
            }
        }
        layer
            .set_mask(&reprojected)
            .map_err(|error| error.to_string())?;
        let source = layer
            .style_source
            .as_ref()
            .ok_or_else(|| format!("layer {} has no retail style provenance", layer.id))?;
        let template = templates
            .iter()
            .find(|template| {
                template.stage_id == source.stage_id && template.layer_index == source.layer_index
            })
            .ok_or_else(|| format!("retail template {} is unavailable", source.display_name))?;
        let model = read_stage_asset_bytes(&template.model_asset_path)
            .and_then(J3dRebuildDocument::parse)
            .map_err(|error| error.to_string())?;
        layer.generated_model = Some(
            generate_floor_pollution_model(
                &model,
                render_triangles,
                layer.region,
                width as u16,
                height as u16,
                source.forced_incompatible,
            )
            .map_err(|error| error.to_string())?,
        );
    }
    authoring.terrain_fingerprint = terrain_fingerprint;
    authoring.format_version = GOOP_AUTHORING_FORMAT_VERSION;
    authoring.stale = false;
    document
        .compile_goop_authoring()
        .map_err(|error| error.to_string())
}

fn copy_template_animations(
    document: &mut StageDocument,
    template: &RetailGoopTemplate,
    target_stem: &str,
) -> Result<(), String> {
    let bytes = fs::read(&template.archive_path).map_err(|error| error.to_string())?;
    let archive = SourceFreeStageArchive::parse(&bytes).map_err(|error| error.to_string())?;
    for resource in archive.resources() {
        let path = String::from_utf8_lossy(&resource.raw_path).replace('\\', "/");
        let candidate = Path::new(&path);
        let extension = candidate
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let stem_matches = candidate
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(&template.resource_stem));
        if stem_matches && matches!(extension.as_str(), "btk" | "btp" | "brk" | "bck" | "bas") {
            document.upsert_authored_resource(
                format!("map/pollution/{target_stem}.{extension}").into_bytes(),
                resource.document.clone(),
            );
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct GoopBrush {
    radius: f32,
    hardness: f32,
    opacity: f32,
    erase: bool,
}

fn paint_brush_sample(
    layer: &GoopLayerAuthoring,
    mask: &mut [u8],
    sample: [f32; 3],
    brush: GoopBrush,
    changes: &mut Vec<(usize, u8, u8)>,
) {
    let Ok((width, height)) = layer.dimensions() else {
        return;
    };
    let GoopBrush {
        radius,
        hardness,
        opacity,
        erase,
    } = brush;
    let cell_size = layer.runtime.vertical_scale;
    if !cell_size.is_finite() || cell_size <= 0.0 {
        return;
    }
    let min_x = (((sample[0] - radius - layer.region.min_x) / cell_size).floor() as isize)
        .clamp(0, width.saturating_sub(1) as isize) as usize;
    let max_x = (((sample[0] + radius - layer.region.min_x) / cell_size).ceil() as isize)
        .clamp(0, width.saturating_sub(1) as isize) as usize;
    let min_y = (((sample[2] - radius - layer.region.min_z) / cell_size).floor() as isize)
        .clamp(0, height.saturating_sub(1) as isize) as usize;
    let max_y = (((sample[2] + radius - layer.region.min_z) / cell_size).ceil() as isize)
        .clamp(0, height.saturating_sub(1) as isize) as usize;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if !layer.valid_cell(x, y) {
                continue;
            }
            let world_x = layer.region.min_x + (x as f32 + 0.5) * cell_size;
            let world_z = layer.region.min_z + (y as f32 + 0.5) * cell_size;
            let normalized = (world_x - sample[0]).hypot(world_z - sample[2]) / radius.max(1.0);
            if normalized > 1.0 {
                continue;
            }
            let feather = if normalized <= hardness {
                1.0
            } else {
                1.0 - (normalized - hardness) / (1.0 - hardness).max(0.0001)
            };
            let strength = (feather * opacity).clamp(0.0, 1.0);
            let index = y * width + x;
            let before = mask[index];
            let after = if erase {
                (f32::from(before) * (1.0 - strength)).round() as u8
            } else {
                (f32::from(before) + (255.0 - f32::from(before)) * strength).round() as u8
            };
            if before != after {
                mask[index] = after;
                changes.push((index, before, after));
            }
        }
    }
}

fn flood_fill(
    layer: &GoopLayerAuthoring,
    mask: &mut [u8],
    start: [usize; 2],
    erase: bool,
    changes: &mut Vec<(usize, u8, u8)>,
) {
    let Ok((width, height)) = layer.dimensions() else {
        return;
    };
    let [start_x, start_y] = start;
    if !layer.valid_cell(start_x, start_y) {
        return;
    }
    let mut pending = VecDeque::from([(start_x, start_y)]);
    let mut visited = vec![false; width * height];
    let value = if erase { 0 } else { 255 };
    while let Some((x, y)) = pending.pop_front() {
        let index = y * width + x;
        if visited[index] || !layer.valid_cell(x, y) {
            continue;
        }
        visited[index] = true;
        let before = mask[index];
        if before != value {
            mask[index] = value;
            changes.push((index, before, value));
        }
        if x > 0 {
            pending.push_back((x - 1, y));
        }
        if x + 1 < width {
            pending.push_back((x + 1, y));
        }
        if y > 0 {
            pending.push_back((x, y - 1));
        }
        if y + 1 < height {
            pending.push_back((x, y + 1));
        }
    }
}

fn coalesce_pixel_changes(changes: BTreeMap<usize, (u8, u8)>) -> Vec<GoopPixelSpan> {
    let mut spans: Vec<GoopPixelSpan> = Vec::new();
    for (index, (before, after)) in changes {
        if let Some(span) = spans.last_mut() {
            if span.start + span.before.len() == index {
                span.before.push(before);
                span.after.push(after);
                continue;
            }
        }
        spans.push(GoopPixelSpan {
            start: index,
            before: vec![before],
            after: vec![after],
        });
    }
    spans
}

fn ray_triangle_distance(
    origin: [f32; 3],
    direction: [f32; 3],
    vertices: [[f32; 3]; 3],
) -> Option<f32> {
    let edge1 = vec3_sub(vertices[1], vertices[0]);
    let edge2 = vec3_sub(vertices[2], vertices[0]);
    let p = vec3_cross(direction, edge2);
    let determinant = vec3_dot(edge1, p);
    if determinant.abs() < 0.000001 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let t = vec3_sub(origin, vertices[0]);
    let u = vec3_dot(t, p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = vec3_cross(t, edge1);
    let v = vec3_dot(direction, q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = vec3_dot(edge2, q) * inverse;
    (distance > 0.0).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editable_layer() -> GoopLayerAuthoring {
        GoopLayerAuthoring {
            id: "test".to_string(),
            runtime_index: 0,
            origin: GoopLayerOrigin::Generated,
            plane: GoopPlane::Floor,
            behavior: GoopBehavior::Normal,
            visible: true,
            region: sms_scene::GoopRegion {
                min_x: 0.0,
                min_z: 0.0,
                max_x: 320.0,
                max_z: 160.0,
            },
            runtime: YmpLayer {
                layer_type: 0,
                subtype: 0,
                flags: 0,
                reserved: 0,
                vertical_offset: 0.0,
                vertical_scale: 40.0,
                min_x: 0.0,
                min_z: 0.0,
                max_x: 320.0,
                max_z: 160.0,
                width_log2: 3,
                height_log2: 2,
                user_value: 0,
                map_offset: 0,
                depth_map: vec![0; 32],
            },
            bitmap: Some(BmpFile::new_pollution_mask(8, 4, vec![0; 32]).unwrap()),
            generated_model: None,
            style_source: None,
            resource_stem: "pollution00".to_string(),
            metadata_dirty: false,
        }
    }

    #[test]
    fn legacy_generated_layers_request_exactly_one_automatic_rebuild() {
        let mut authoring = GoopAuthoringDocument {
            format_version: GOOP_AUTHORING_FORMAT_VERSION - 1,
            layers: vec![editable_layer()],
            terrain_fingerprint: 0,
            stale: false,
        };
        assert!(generated_goop_requires_upgrade(&authoring));
        assert!(authoring
            .validate()
            .unwrap_err()
            .to_string()
            .contains("authoring format version"));

        authoring.format_version = GOOP_AUTHORING_FORMAT_VERSION;
        assert!(!generated_goop_requires_upgrade(&authoring));

        authoring.format_version = GOOP_AUTHORING_FORMAT_VERSION - 1;
        authoring.layers[0].origin = GoopLayerOrigin::Imported;
        assert!(!generated_goop_requires_upgrade(&authoring));
    }

    #[test]
    fn pixel_changes_are_coalesced_into_contiguous_spans() {
        let changes = BTreeMap::from([(2, (0, 1)), (3, (2, 3)), (7, (4, 5))]);
        let spans = coalesce_pixel_changes(changes);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].start, 2);
        assert_eq!(spans[0].before, vec![0, 2]);
        assert_eq!(spans[0].after, vec![1, 3]);
    }

    #[test]
    fn collision_raycast_hits_triangle() {
        let distance = ray_triangle_distance(
            [0.25, 1.0, 0.25],
            [0.0, -1.0, 0.0],
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        );
        assert_eq!(distance, Some(1.0));
    }

    #[test]
    fn render_surface_filter_keeps_floors_and_rejects_walls_and_undersides() {
        let floor = [[0.0, 25.0, 0.0], [100.0, 25.0, 0.0], [0.0, 25.0, 100.0]];
        assert!(render_triangle_is_upward_facing(
            floor,
            Some([[0.0, 1.0, 0.0]; 3])
        ));
        assert!(render_triangle_is_upward_facing(floor, None));

        let wall = [[0.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 0.0, 100.0]];
        assert!(!render_triangle_is_upward_facing(
            wall,
            Some([[1.0, 0.0, 0.0]; 3])
        ));
        assert!(!render_triangle_is_upward_facing(wall, None));

        assert!(!render_triangle_is_upward_facing(
            floor,
            Some([[0.0, -1.0, 0.0]; 3])
        ));
        let reversed_floor = [floor[0], floor[2], floor[1]];
        assert!(!render_triangle_is_upward_facing(reversed_floor, None));
    }

    #[test]
    fn render_surface_filter_requires_the_topmost_collision_floor() {
        let floor = [[0.0, 25.0, 0.0], [100.0, 25.0, 0.0], [0.0, 25.0, 100.0]];
        let mut collision = vec![GoopTerrainTriangle { vertices: floor }];
        assert!(render_triangle_matches_topmost_collision(floor, &collision));

        let floating = floor.map(|mut vertex| {
            vertex[1] += 31.0;
            vertex
        });
        assert!(!render_triangle_matches_topmost_collision(
            floating, &collision
        ));

        let upper_floor = floor.map(|mut vertex| {
            vertex[1] += 100.0;
            vertex
        });
        collision.push(GoopTerrainTriangle {
            vertices: upper_floor,
        });
        assert!(!render_triangle_matches_topmost_collision(
            floor, &collision
        ));
        assert!(render_triangle_matches_topmost_collision(
            upper_floor,
            &collision
        ));
    }

    #[test]
    fn brush_combines_opacity_and_rejects_invalid_cells() {
        let mut layer = editable_layer();
        layer.runtime.set_depth(2, 1, 0xff).unwrap();
        let mut mask = vec![0; 32];
        let mut changes = Vec::new();
        paint_brush_sample(
            &layer,
            &mut mask,
            [60.0, 0.0, 60.0],
            GoopBrush {
                radius: 100.0,
                hardness: 1.0,
                opacity: 0.5,
                erase: false,
            },
            &mut changes,
        );
        assert_eq!(mask[1 + 8], 128);
        assert_eq!(mask[2 + 8], 0);
        assert!(!changes.is_empty());
    }

    #[test]
    fn retail_scale_controls_world_cell_mapping_and_brush_centers() {
        let mut layer = editable_layer();
        layer.runtime.vertical_scale = 32.0;
        layer.region.max_x = 256.0;
        layer.region.max_z = 128.0;
        assert_eq!(layer.world_to_cell(80.0, 48.0), Some((2, 1)));

        let mut mask = vec![0; 32];
        let mut changes = Vec::new();
        paint_brush_sample(
            &layer,
            &mut mask,
            [80.0, 0.0, 48.0],
            GoopBrush {
                radius: 10.0,
                hardness: 1.0,
                opacity: 1.0,
                erase: false,
            },
            &mut changes,
        );
        assert_eq!(mask[10], 255);
        assert_eq!(changes, vec![(10, 0, 255)]);
    }

    #[test]
    fn connected_fill_stops_at_invalid_depth_cells() {
        let mut layer = editable_layer();
        for y in 0..4 {
            layer.runtime.set_depth(4, y, 0xff).unwrap();
        }
        let mut mask = vec![0; 32];
        let mut changes = Vec::new();
        flood_fill(&layer, &mut mask, [1, 1], false, &mut changes);
        assert!(mask
            .chunks_exact(8)
            .all(|row| row[..4].iter().all(|value| *value == 255)));
        assert!(mask
            .chunks_exact(8)
            .all(|row| row[4..].iter().all(|value| *value == 0)));
    }

    #[test]
    fn overlay_heights_decode_runtime_depth_and_anchor_invalid_boundaries() {
        let mut layer = editable_layer();
        layer.runtime.vertical_offset = -80.0;
        layer.runtime.vertical_scale = 32.0;
        layer.runtime.set_depth(2, 1, 3).unwrap();
        assert_eq!(goop_cell_surface_y(&layer, 2, 1), Some(16.0));

        layer.runtime.set_depth(3, 1, 0xff).unwrap();
        assert_eq!(goop_invalid_marker_surface_y(&layer, 3, 1), Some(16.0));

        for y in 0..4 {
            for x in 0..8 {
                layer.runtime.set_depth(x, y, 0xff).unwrap();
            }
        }
        assert_eq!(goop_invalid_marker_surface_y(&layer, 3, 1), None);
    }

    #[test]
    #[ignore = "requires SMS_BASE_ROOT with extracted retail stage archives"]
    fn retail_floor_template_census_finds_safe_templates() {
        let root = std::env::var_os("SMS_BASE_ROOT")
            .expect("set SMS_BASE_ROOT to an extracted retail game root");
        let archives =
            sms_formats::discover_scene_archives(root).expect("discover retail scene archives");
        let (templates, warnings) = index_retail_goop_templates(&archives);
        let compatible = templates
            .iter()
            .filter(|template| template.compatible)
            .collect::<Vec<_>>();
        assert!(
            !compatible.is_empty(),
            "retail census found no structurally safe floor-goop templates; warnings: {warnings:#?}"
        );
        for template in compatible {
            assert!(template.archive_path.is_file());
            assert!(!template.resource_stem.is_empty());
            let model = read_stage_asset_bytes(&template.model_asset_path)
                .and_then(J3dRebuildDocument::parse)
                .expect("reparse compatible retail goop template");
            assert!(model
                .sections
                .iter()
                .any(|section| { matches!(section.data, J3dRebuildSectionData::Materials(_)) }));
            assert_eq!(
                model.sections.iter().find_map(|section| {
                    if let J3dRebuildSectionData::Textures(textures) = &section.data {
                        textures.textures.first().map(|texture| texture.format)
                    } else {
                        None
                    }
                }),
                Some(1)
            );

            let region = sms_scene::GoopRegion {
                min_x: 0.0,
                min_z: 0.0,
                max_x: 320.0,
                max_z: 160.0,
            };
            // Feed the clockwise GX winding and the upward vertex normals
            // decoded from the finalized map-render BMD.
            let terrain = vec![GoopRenderTriangle {
                vertices: [[0.0, 0.0, 0.0], [320.0, 0.0, 0.0], [0.0, 0.0, 160.0]],
                normals: Some([[0.0, 1.0, 0.0]; 3]),
            }];
            let generated = generate_floor_pollution_model(&model, &terrain, region, 8, 4, false)
                .expect("generate with compatible retail material template");
            let preview = sms_formats::J3dFile::parse(
                generated
                    .to_bytes()
                    .expect("encode generated pollution BMD"),
            )
            .expect("parse generated pollution BMD")
            .geometry_preview()
            .expect("preview generated pollution BMD");
            assert!(!preview.triangles.is_empty());
            assert!(preview.triangles.iter().all(|triangle| {
                let [a, b, c] = triangle.vertices;
                let normal_y = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
                normal_y < 0.0 && triangle.cull_mode == Some(2)
            }));
            assert!(
                preview.triangles.iter().any(|triangle| {
                    triangle.texture_index == Some(0) || triangle.mask_texture_index == Some(0)
                }),
                "compatible template {} / {} does not bind mutable texture zero",
                template.stage_id,
                template.resource_stem
            );
        }
    }
}
