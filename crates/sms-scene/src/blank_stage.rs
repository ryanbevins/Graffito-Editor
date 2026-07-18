//! Typed, source-free construction of a minimal playable stage archive.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sms_formats::{
    ColFile, FormatError, J3dRebuildDocument, JDramaDocument, JDramaField, JDramaFieldValue,
    JDramaLightMap, JDramaRecord, JDramaRecordPayload, JDramaTransform,
};

use crate::stage_archive::parse_resource;
use crate::{
    Result, SceneError, SourceFreeStageArchive, StageCompression, StageResource,
    StageResourceDocument,
};

pub const BLANK_STAGE_PRESET_VERSION: u32 = 1;
pub const DEFAULT_BLANK_STAGE_TARGET_SLOT: &str = "test11";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlankStageBootstrapKind {
    Model,
    Collision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlankStageBootstrapRequirement {
    pub raw_path: &'static [u8],
    pub kind: BlankStageBootstrapKind,
}

pub const BLANK_STAGE_BOOTSTRAP_REQUIREMENTS: [BlankStageBootstrapRequirement; 5] = [
    BlankStageBootstrapRequirement {
        raw_path: b"mapobj/coin.bmd",
        kind: BlankStageBootstrapKind::Model,
    },
    BlankStageBootstrapRequirement {
        raw_path: b"mapobj/bottle_large.bmd",
        kind: BlankStageBootstrapKind::Model,
    },
    BlankStageBootstrapRequirement {
        raw_path: b"mapobj/normalblock.bmd",
        kind: BlankStageBootstrapKind::Model,
    },
    BlankStageBootstrapRequirement {
        raw_path: b"mapobj/normalblock.col",
        kind: BlankStageBootstrapKind::Collision,
    },
    BlankStageBootstrapRequirement {
        raw_path: b"mapobj/juiceblock.bmd",
        kind: BlankStageBootstrapKind::Model,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlankStageBootstrapResource {
    pub raw_path: Vec<u8>,
    pub bytes: Vec<u8>,
}

/// Parsed bootstrap dependency closure. Input bytes are consumed into typed
/// resource documents immediately and are never retained as archive fallbacks.
#[derive(Debug, Clone, PartialEq)]
pub struct BlankStageBootstrapManifest {
    resources: Vec<StageResource>,
}

impl BlankStageBootstrapManifest {
    pub fn from_authored_bytes(
        resources: impl IntoIterator<Item = BlankStageBootstrapResource>,
    ) -> Result<Self> {
        let mut seen = BTreeSet::new();
        let mut parsed = Vec::new();
        for resource in resources {
            let raw_path = normalize_bootstrap_path(resource.raw_path)?;
            if !seen.insert(raw_path.clone()) {
                return Err(blank_stage_error(format!(
                    "bootstrap resource {} is duplicated",
                    display_raw_path(&raw_path)
                )));
            }
            let document = parse_resource(&raw_path, &resource.bytes).map_err(|source| {
                SceneError::StageResource {
                    path: display_raw_path(&raw_path),
                    source,
                }
            })?;
            parsed.push(StageResource { raw_path, document });
        }
        parsed.sort_by(|left, right| left.raw_path.cmp(&right.raw_path));

        for requirement in BLANK_STAGE_BOOTSTRAP_REQUIREMENTS {
            let Some(resource) = parsed
                .iter()
                .find(|resource| resource.raw_path == requirement.raw_path)
            else {
                return Err(blank_stage_error(format!(
                    "required authored bootstrap resource {} is missing",
                    display_raw_path(requirement.raw_path)
                )));
            };
            let kind_matches = matches!(
                (requirement.kind, &resource.document),
                (
                    BlankStageBootstrapKind::Model,
                    StageResourceDocument::Model(_)
                ) | (
                    BlankStageBootstrapKind::Collision,
                    StageResourceDocument::Collision(_)
                )
            );
            if !kind_matches {
                return Err(blank_stage_error(format!(
                    "bootstrap resource {} has the wrong semantic kind",
                    display_raw_path(requirement.raw_path)
                )));
            }
        }
        Ok(Self { resources: parsed })
    }

    pub fn resources(&self) -> &[StageResource] {
        &self.resources
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlankStageTargetMetadata {
    pub target_slot: String,
    pub output_archive_name: String,
    pub replaces_existing_stage_mapping: bool,
    pub runtime_patch_required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlankStagePreset {
    pub target_slot: String,
    pub spawn: Option<JDramaTransform>,
    pub compression: Option<StageCompression>,
}

impl Default for BlankStagePreset {
    fn default() -> Self {
        Self {
            target_slot: DEFAULT_BLANK_STAGE_TARGET_SLOT.to_string(),
            spawn: None,
            compression: Some(StageCompression::Yaz0 { reserved: [0; 8] }),
        }
    }
}

impl BlankStagePreset {
    pub fn target_metadata(&self) -> Result<BlankStageTargetMetadata> {
        validate_target_slot(&self.target_slot)?;
        Ok(BlankStageTargetMetadata {
            target_slot: self.target_slot.clone(),
            output_archive_name: format!("{}.szs", self.target_slot),
            replaces_existing_stage_mapping: true,
            runtime_patch_required: false,
        })
    }

    /// Builds the minimum typed stage shell around caller-authored world and
    /// bootstrap resources. No resource is copied from a retail archive.
    pub fn build(
        &self,
        world_model: J3dRebuildDocument,
        world_collision: ColFile,
        bootstrap: BlankStageBootstrapManifest,
    ) -> Result<SourceFreeStageArchive> {
        validate_target_slot(&self.target_slot)?;
        let spawn = match self.spawn {
            Some(spawn) => {
                validate_transform(spawn)?;
                spawn
            }
            None => derive_spawn(&world_collision).ok_or_else(|| {
                blank_stage_error(
                    "world collision has no upward nondegenerate face; choose a spawn manually",
                )
            })?,
        };
        let placement = blank_scene_document(spawn)?;

        let mut archive = SourceFreeStageArchive::new_for_blank(
            self.target_slot.clone(),
            BLANK_STAGE_PRESET_VERSION,
        )?;
        archive.set_compression(self.compression);
        archive.insert_resource(
            b"map/scene.bin".to_vec(),
            StageResourceDocument::Placement(placement),
        )?;
        archive.insert_resource(
            b"map/map/map.bmd".to_vec(),
            StageResourceDocument::Model(world_model),
        )?;
        archive.insert_resource(
            b"map/map.col".to_vec(),
            StageResourceDocument::Collision(world_collision),
        )?;
        for resource in bootstrap.resources {
            archive.insert_resource(resource.raw_path, resource.document)?;
        }
        Ok(archive)
    }
}

fn blank_scene_document(spawn: JDramaTransform) -> Result<JDramaDocument> {
    let conductor = group(
        "GroupObj",
        "\u{30B3}\u{30F3}\u{30C0}\u{30AF}\u{30BF}\u{30FC}\u{521D}\u{671F}\u{5316}\u{7528}",
        Vec::new(),
        vec![
            fields_record(
                "ItemManager",
                "\u{30A2}\u{30A4}\u{30C6}\u{30E0}\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC}",
                vec![
                    field(
                        "character_name",
                        JDramaFieldValue::String(
                            "\u{30A2}\u{30A4}\u{30C6}\u{30E0}\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC} \u{30AD}\u{30E3}\u{30E9}"
                                .to_string(),
                        ),
                    ),
                    field("capacity", JDramaFieldValue::U32(300)),
                    field("clip_distance", JDramaFieldValue::F32(12_000.0)),
                    field("clip_radius", JDramaFieldValue::F32(500.0)),
                ],
            )?,
            fields_record(
                "MapObjManager",
                "\u{5730}\u{5F62}\u{30AA}\u{30D6}\u{30B8}\u{30A7}\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC}",
                vec![
                    field(
                        "character_name",
                        JDramaFieldValue::String(
                            "\u{30A2}\u{30A4}\u{30C6}\u{30E0}\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC} \u{30AD}\u{30E3}\u{30E9}"
                                .to_string(),
                        ),
                    ),
                    field("capacity", JDramaFieldValue::U32(300)),
                    field("clip_distance", JDramaFieldValue::F32(5_000.0)),
                    field("clip_radius", JDramaFieldValue::F32(500.0)),
                ],
            )?,
        ],
    )?;
    let mirror_scene = group(
        "GroupObj",
        "\u{93E1}\u{30B7}\u{30FC}\u{30F3}",
        Vec::new(),
        vec![actor(
            "MirrorCamera",
            "\u{93E1}\u{30AB}\u{30E1}\u{30E9}",
            identity_transform(),
            "\u{93E1}\u{30AB}\u{30E1}\u{30E9} \u{30AD}\u{30E3}\u{30E9}",
            Vec::new(),
        )?],
    )?;
    let mirror_manager = fields_record(
        "MirrorModelManager",
        "\u{93E1}\u{8868}\u{793A}\u{30E2}\u{30C7}\u{30EB}\u{7BA1}\u{7406}",
        vec![
            field("opaque_model_count", JDramaFieldValue::I32(0)),
            field("translucent_model_count", JDramaFieldValue::I32(0)),
            field("paired_model_count", JDramaFieldValue::I32(0)),
        ],
    )?;

    let normal_scene = group(
        "MarScene",
        "\u{901A}\u{5E38}\u{30B7}\u{30FC}\u{30F3}",
        vec![field(
            "light_map",
            JDramaFieldValue::LightMap(JDramaLightMap::default()),
        )],
        vec![
            ambient_group()?,
            light_group()?,
            strategy(spawn)?,
            camera_group()?,
        ],
    )?;
    Ok(JDramaDocument {
        root: group(
            "GroupObj",
            "\u{5168}\u{4F53}\u{30B7}\u{30FC}\u{30F3}",
            Vec::new(),
            vec![
                conductor,
                mirror_scene,
                mirror_manager,
                group(
                    "GroupObj",
                    "\u{30B9}\u{30DA}\u{30AD}\u{30E5}\u{30E9}\u{30B7}\u{30FC}\u{30F3}",
                    Vec::new(),
                    Vec::new(),
                )?,
                group(
                    "GroupObj",
                    "\u{30A4}\u{30F3}\u{30C0}\u{30A4}\u{30EC}\u{30AF}\u{30C8}\u{30B7}\u{30FC}\u{30F3}",
                    Vec::new(),
                    Vec::new(),
                )?,
                normal_scene,
            ],
        )?,
    })
}

fn ambient_group() -> Result<JDramaRecord> {
    let roles = [
        "\u{30D7}\u{30EC}\u{30A4}\u{30E4}\u{30FC}",
        "\u{30AA}\u{30D6}\u{30B8}\u{30A7}\u{30AF}\u{30C8}",
        "\u{6575}",
    ];
    let mut children = Vec::with_capacity(6);
    for role in roles {
        children.push(fields_record(
            "AmbColor",
            &format!("\u{592A}\u{967D}\u{30A2}\u{30F3}\u{30D3}\u{30A8}\u{30F3}\u{30C8}\u{FF08}{role}\u{FF09}"),
            vec![field(
                "color",
                JDramaFieldValue::ColorRgba8([100, 100, 100, 255]),
            )],
        )?);
        children.push(fields_record(
            "AmbColor",
            &format!(
                "\u{5F71}\u{30A2}\u{30F3}\u{30D3}\u{30A8}\u{30F3}\u{30C8}\u{FF08}{role}\u{FF09}"
            ),
            vec![field(
                "color",
                JDramaFieldValue::ColorRgba8([40, 40, 40, 255]),
            )],
        )?);
    }
    group("AmbAry", "Ambient Group", Vec::new(), children)
}

fn light_group() -> Result<JDramaRecord> {
    let roles = [
        "\u{30D7}\u{30EC}\u{30A4}\u{30E4}\u{30FC}",
        "\u{30AA}\u{30D6}\u{30B8}\u{30A7}\u{30AF}\u{30C8}",
        "\u{6575}",
    ];
    let mut children = Vec::with_capacity(15);
    for role in roles {
        children.push(light(
            &format!("\u{592A}\u{967D}\u{FF08}{role}\u{FF09}"),
            [-100_000.0, 300_000.0, 400_000.0],
            [200, 200, 200, 255],
        )?);
        children.push(light(
            &format!("\u{592A}\u{967D}\u{30B5}\u{30D6}\u{FF08}{role}\u{FF09}"),
            [100_000.0, -300_000.0, -400_000.0],
            [50, 50, 50, 255],
        )?);
        children.push(light(
            &format!("\u{5F71}\u{FF08}{role}\u{FF09}"),
            [-100_000.0, 300_000.0, 400_000.0],
            [80, 80, 80, 255],
        )?);
        children.push(light(
            &format!("\u{5F71}\u{30B5}\u{30D6}\u{FF08}{role}\u{FF09}"),
            [100_000.0, -300_000.0, -400_000.0],
            [0, 0, 0, 255],
        )?);
        children.push(light(
            &format!(
                "\u{592A}\u{967D}\u{30B9}\u{30DA}\u{30AD}\u{30E5}\u{30E9}\u{FF08}{role}\u{FF09}"
            ),
            [-100_000.0, 300_000.0, 400_000.0],
            [200, 200, 200, 255],
        )?);
    }
    group("LightAry", "Light Group", Vec::new(), children)
}

fn light(name: &str, position: [f32; 3], color: [u8; 4]) -> Result<JDramaRecord> {
    fields_record(
        "Light",
        name,
        vec![
            field("position", JDramaFieldValue::Vec3F32(position)),
            field("color", JDramaFieldValue::ColorRgba8(color)),
            field("range", JDramaFieldValue::F32(50.0)),
        ],
    )
}

fn strategy(spawn: JDramaTransform) -> Result<JDramaRecord> {
    let map = fields_record(
        "Map",
        "\u{30DE}\u{30C3}\u{30D7}",
        vec![
            field("translucent_group_count", JDramaFieldValue::U32(0)),
            field("collision_grid_width", JDramaFieldValue::I32(80)),
            field("collision_grid_height", JDramaFieldValue::I32(80)),
            field("collision_triangle_capacity", JDramaFieldValue::I32(12_000)),
            field("collision_list_capacity", JDramaFieldValue::I32(25_000)),
            field("collision_warp_capacity", JDramaFieldValue::I32(8_000)),
            field("warp_pair_count", JDramaFieldValue::U32(0)),
        ],
    )?;
    let manager_children = vec![
        fields_record(
            "SunMgr",
            "\u{592A}\u{967D}\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC}",
            vec![
                field("sun_color_r", JDramaFieldValue::U32(10_103)),
                field("sun_color_g", JDramaFieldValue::U32(255)),
                field("sun_color_b", JDramaFieldValue::U32(120)),
                field("sun_color_a", JDramaFieldValue::U32(195)),
                field("sun_size", JDramaFieldValue::F32(0.0)),
            ],
        )?,
        empty_record(
            "CubeCamera",
            "\u{30AD}\u{30E5}\u{30FC}\u{30D6}\u{FF08}\u{30AB}\u{30E1}\u{30E9}\u{FF09}",
        )?,
        empty_record(
            "CubeMirror",
            "\u{30AD}\u{30E5}\u{30FC}\u{30D6}\u{FF08}\u{93E1}\u{FF09}",
        )?,
        empty_record(
            "CubeWire",
            "\u{30AD}\u{30E5}\u{30FC}\u{30D6}\u{FF08}\u{30EF}\u{30A4}\u{30E4}\u{30FC}\u{FF09}",
        )?,
        empty_record(
            "CubeArea",
            "\u{30AD}\u{30E5}\u{30FC}\u{30D6}\u{FF08}\u{30A8}\u{30EA}\u{30A2}\u{FF09}",
        )?,
        fields_record(
            "MapWireManager",
            "\u{30EF}\u{30A4}\u{30E4}\u{30FC}\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC}",
            vec![
                field(
                    "character_name",
                    JDramaFieldValue::String(
                        "\u{30EF}\u{30A4}\u{30E4}\u{30FC}\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC} \u{30AD}\u{30E3}\u{30E9}"
                            .to_string(),
                    ),
                ),
                field("wire_capacity", JDramaFieldValue::U32(200)),
                field("actor_capacity", JDramaFieldValue::U32(10)),
                field("draw_width", JDramaFieldValue::F32(10.0)),
                field("draw_height", JDramaFieldValue::F32(20.0)),
                field("upper_red", JDramaFieldValue::U32(200)),
                field("upper_green", JDramaFieldValue::U32(200)),
                field("upper_blue", JDramaFieldValue::U32(200)),
                field("lower_red", JDramaFieldValue::U32(128)),
                field("lower_green", JDramaFieldValue::U32(128)),
                field("lower_blue", JDramaFieldValue::U32(128)),
            ],
        )?,
    ];
    let mario = actor(
        "Mario",
        "\u{30DE}\u{30EA}\u{30AA}",
        spawn,
        "\u{30DE}\u{30EA}\u{30AA} \u{30AD}\u{30E3}\u{30E9}",
        vec![
            field("starting_water", JDramaFieldValue::U32(100)),
            field("equipment_flags", JDramaFieldValue::U32(0)),
        ],
    )?;
    let pollution = actor(
        "Pollution",
        "\u{843D}\u{66F8}\u{304D}\u{7BA1}\u{7406}",
        identity_transform(),
        "\u{843D}\u{66F8}\u{304D}\u{7BA1}\u{7406} \u{30AD}\u{30E3}\u{30E9}",
        Vec::new(),
    )?;

    group(
        "Strategy",
        "\u{30B9}\u{30C8}\u{30E9}\u{30C6}\u{30B8}",
        Vec::new(),
        vec![
            indexed_group(
                0,
                "\u{30DE}\u{30C3}\u{30D7}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                vec![map],
            )?,
            indexed_group(
                1,
                "\u{7A7A}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                Vec::new(),
            )?,
            indexed_group(
                2,
                "\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                manager_children,
            )?,
            indexed_group(
                3,
                "\u{30AA}\u{30D6}\u{30B8}\u{30A7}\u{30AF}\u{30C8}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                Vec::new(),
            )?,
            indexed_group(
                4,
                "\u{843D}\u{66F8}\u{304D}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                vec![pollution],
            )?,
            indexed_group(
                5,
                "\u{30A2}\u{30A4}\u{30C6}\u{30E0}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                Vec::new(),
            )?,
            indexed_group(
                6,
                "\u{30D7}\u{30EC}\u{30FC}\u{30E4}\u{30FC}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                vec![mario],
            )?,
            indexed_group(
                7,
                "\u{6575}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                Vec::new(),
            )?,
            indexed_group(
                8,
                "\u{30DC}\u{30B9}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                Vec::new(),
            )?,
            indexed_group(
                9,
                "\u{FF2E}\u{FF30}\u{FF23}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                Vec::new(),
            )?,
            indexed_group(
                10,
                "\u{6C34}\u{30D1}\u{30FC}\u{30C6}\u{30A3}\u{30AF}\u{30EB}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                Vec::new(),
            )?,
            indexed_group(
                11,
                "\u{521D}\u{671F}\u{5316}\u{7528}\u{30B0}\u{30EB}\u{30FC}\u{30D7}",
                Vec::new(),
            )?,
        ],
    )
}

fn camera_group() -> Result<JDramaRecord> {
    group(
        "GroupObj",
        "Cameras",
        Vec::new(),
        vec![empty_record("PolarSubCamera", "camera 1")?],
    )
}

fn indexed_group(index: u32, name: &str, children: Vec<JDramaRecord>) -> Result<JDramaRecord> {
    group(
        "IdxGroup",
        name,
        vec![field("group_index", JDramaFieldValue::U32(index))],
        children,
    )
}

fn group(
    type_name: &str,
    name: &str,
    fields: Vec<JDramaField>,
    children: Vec<JDramaRecord>,
) -> Result<JDramaRecord> {
    Ok(JDramaRecord::new(
        type_name,
        name,
        JDramaRecordPayload::Group { fields, children },
    )?)
}

fn fields_record(type_name: &str, name: &str, fields: Vec<JDramaField>) -> Result<JDramaRecord> {
    Ok(JDramaRecord::new(
        type_name,
        name,
        JDramaRecordPayload::Fields { fields },
    )?)
}

fn empty_record(type_name: &str, name: &str) -> Result<JDramaRecord> {
    Ok(JDramaRecord::new(
        type_name,
        name,
        JDramaRecordPayload::Empty,
    )?)
}

fn actor(
    type_name: &str,
    name: &str,
    transform: JDramaTransform,
    character_name: &str,
    fields: Vec<JDramaField>,
) -> Result<JDramaRecord> {
    Ok(JDramaRecord::new(
        type_name,
        name,
        JDramaRecordPayload::Actor {
            transform,
            character_name: character_name.to_string(),
            light_map: JDramaLightMap::default(),
            fields,
        },
    )?)
}

fn field(name: &str, value: JDramaFieldValue) -> JDramaField {
    JDramaField {
        name: name.to_string(),
        value,
    }
}

fn identity_transform() -> JDramaTransform {
    JDramaTransform {
        translation: [0.0; 3],
        rotation: [0.0; 3],
        scale: [1.0; 3],
    }
}

fn derive_spawn(collision: &ColFile) -> Option<JDramaTransform> {
    let vertices = collision.vertices();
    let mut best: Option<(f32, [f32; 3])> = None;
    for group in collision.groups() {
        if !is_spawnable_surface_type(group.surface_type) {
            continue;
        }
        for triangle in &group.triangles {
            let [a, b, c] = triangle
                .vertex_indices
                .map(|index| vertices.get(index as usize).map(|vertex| vertex.position));
            let (Some(a), Some(b), Some(c)) = (a, b, c) else {
                continue;
            };
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal_y = ab[2] * ac[0] - ab[0] * ac[2];
            if !normal_y.is_finite() || normal_y <= f32::EPSILON {
                continue;
            }
            let center = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0 + 100.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            if !center.into_iter().all(f32::is_finite) {
                continue;
            }
            let distance_squared = center[0] * center[0] + center[2] * center[2];
            if best
                .as_ref()
                .is_none_or(|(best_distance, _)| distance_squared < *best_distance)
            {
                best = Some((distance_squared, center));
            }
        }
    }
    best.map(|(_, translation)| JDramaTransform {
        translation,
        rotation: [0.0; 3],
        scale: [2.0; 3],
    })
}

fn is_spawnable_surface_type(surface_type: u16) -> bool {
    // Strip the shadow and camera-no-clip property flags before applying the
    // decomp's BGTypeBits categories. Water, warp, phase-through, fence, and
    // death surfaces are deliberately excluded from automatic spawn choice.
    let base_type = surface_type & !(0x4000 | 0x8000);
    matches!(base_type, 0x000..=0x00C | 0x109 | 0x500 | 0x701)
}

fn validate_transform(transform: JDramaTransform) -> Result<()> {
    if transform
        .translation
        .into_iter()
        .chain(transform.rotation)
        .chain(transform.scale)
        .all(f32::is_finite)
    {
        Ok(())
    } else {
        Err(blank_stage_error(
            "spawn transform contains non-finite values",
        ))
    }
}

fn validate_target_slot(target_slot: &str) -> Result<()> {
    if target_slot.is_empty()
        || !target_slot
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Err(blank_stage_error(format!(
            "invalid existing stage target slot {target_slot:?}"
        )))
    } else {
        Ok(())
    }
}

fn normalize_bootstrap_path(mut raw_path: Vec<u8>) -> Result<Vec<u8>> {
    if raw_path.first() == Some(&b'/') {
        raw_path.remove(0);
    }
    if raw_path.is_empty()
        || raw_path.contains(&0)
        || raw_path
            .split(|byte| *byte == b'/')
            .any(|component| component.is_empty() || matches!(component, b"." | b".."))
    {
        Err(blank_stage_error(format!(
            "invalid bootstrap archive path {raw_path:02X?}"
        )))
    } else {
        Ok(raw_path)
    }
}

fn display_raw_path(path: &[u8]) -> String {
    path.iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn blank_stage_error(message: impl Into<String>) -> SceneError {
    SceneError::Format(FormatError::Unsupported {
        format: "blank stage preset",
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use sms_formats::{ColGroup, ColTriangle, ColVertex, J3dRebuildDocument, JDramaRecordPayload};

    use super::*;

    fn empty_model() -> J3dRebuildDocument {
        J3dRebuildDocument {
            file_type: *b"bmd3",
            version_tag: *b"SVR3",
            reserved_words: [u32::MAX; 3],
            declared_section_count: 0,
            sections: Vec::new(),
        }
    }

    fn floor_collision() -> ColFile {
        ColFile::new(
            vec![
                ColVertex::new(-100.0, 0.0, -100.0),
                ColVertex::new(0.0, 0.0, 100.0),
                ColVertex::new(100.0, 0.0, -100.0),
            ],
            vec![ColGroup {
                surface_type: 0,
                has_per_triangle_data: false,
                triangles: vec![ColTriangle {
                    vertex_indices: [0, 1, 2],
                    attribute_0: 0,
                    attribute_1: 0,
                    data: None,
                }],
            }],
        )
    }

    fn bootstrap() -> BlankStageBootstrapManifest {
        let model = empty_model().to_bytes().unwrap();
        let collision = floor_collision().encode().unwrap();
        BlankStageBootstrapManifest::from_authored_bytes(BLANK_STAGE_BOOTSTRAP_REQUIREMENTS.map(
            |requirement| BlankStageBootstrapResource {
                raw_path: requirement.raw_path.to_vec(),
                bytes: match requirement.kind {
                    BlankStageBootstrapKind::Model => model.clone(),
                    BlankStageBootstrapKind::Collision => collision.clone(),
                },
            },
        ))
        .unwrap()
    }

    fn collect_type_names<'a>(record: &'a JDramaRecord, output: &mut Vec<&'a str>) {
        output.push(&record.type_name);
        if let JDramaRecordPayload::Group { children, .. } = &record.payload {
            for child in children {
                collect_type_names(child, output);
            }
        }
    }

    fn assert_no_mojibake(record: &JDramaRecord) {
        assert!(!record.type_name.contains('\u{00E3}'));
        assert!(!record.name.contains('\u{00E3}'), "{:?}", record.name);
        let (fields, children): (&[JDramaField], &[JDramaRecord]) = match &record.payload {
            JDramaRecordPayload::Empty => (&[], &[]),
            JDramaRecordPayload::Fields { fields } => (fields, &[]),
            JDramaRecordPayload::Actor {
                character_name,
                fields,
                ..
            } => {
                assert!(!character_name.contains('\u{00E3}'), "{character_name:?}");
                (fields, &[])
            }
            JDramaRecordPayload::Group { fields, children } => (fields, children),
        };
        for field in fields {
            if let JDramaFieldValue::String(value) = &field.value {
                assert!(!value.contains('\u{00E3}'), "{value:?}");
            }
        }
        for child in children {
            assert_no_mojibake(child);
        }
    }

    #[test]
    fn blank_stage_builds_and_reimports_deterministically() {
        let preset = BlankStagePreset::default();
        let archive = preset
            .build(empty_model(), floor_collision(), bootstrap())
            .unwrap();
        assert_eq!(
            archive.origin(),
            &crate::StageOrigin::Blank {
                target_slot: "test11".to_string(),
                preset_version: BLANK_STAGE_PRESET_VERSION,
            }
        );
        for path in [
            b"map/scene.bin".as_slice(),
            b"map/map/map.bmd",
            b"map/map.col",
        ] {
            assert!(archive.resource(path).is_some(), "missing {path:02X?}");
        }
        for requirement in BLANK_STAGE_BOOTSTRAP_REQUIREMENTS {
            assert!(archive.resource(requirement.raw_path).is_some());
        }
        for optional in [
            b"map/scene.ral".as_slice(),
            b"map/tables.bin",
            b"map/startcamera.bck",
            b"map/ymap.ymp",
        ] {
            assert!(archive.resource(optional).is_none());
        }

        let first = archive.encode().unwrap();
        let second = archive.encode().unwrap();
        assert_eq!(first, second);
        let reopened = SourceFreeStageArchive::parse(&first).unwrap();
        assert_eq!(reopened.encode().unwrap(), first);
    }

    #[test]
    fn blank_scene_contains_required_typed_runtime_skeleton() {
        let archive = BlankStagePreset::default()
            .build(empty_model(), floor_collision(), bootstrap())
            .unwrap();
        let StageResourceDocument::Placement(scene) = archive.resource(b"map/scene.bin").unwrap()
        else {
            panic!("scene.bin is not placement data");
        };
        let reparsed = JDramaDocument::parse(&scene.to_bytes().unwrap()).unwrap();
        assert_eq!(reparsed, *scene);
        assert_no_mojibake(&reparsed.root);
        let mut types = Vec::new();
        collect_type_names(&scene.root, &mut types);
        for required in [
            "ItemManager",
            "MapObjManager",
            "MirrorCamera",
            "MirrorModelManager",
            "MarScene",
            "AmbAry",
            "LightAry",
            "Strategy",
            "Map",
            "SunMgr",
            "CubeCamera",
            "CubeArea",
            "MapWireManager",
            "Pollution",
            "Mario",
            "PolarSubCamera",
        ] {
            assert!(types.contains(&required), "missing {required}");
        }
        assert_eq!(types.iter().filter(|name| **name == "IdxGroup").count(), 12);
    }

    #[test]
    fn spawn_is_derived_from_the_nearest_upward_face() {
        let archive = BlankStagePreset::default()
            .build(empty_model(), floor_collision(), bootstrap())
            .unwrap();
        let mario = archive
            .object_placements()
            .into_iter()
            .find(|placement| placement.type_name == "Mario")
            .unwrap();
        assert_eq!(mario.transform.translation, [0.0, 100.0, -100.0 / 3.0]);
        assert_eq!(mario.transform.scale, [2.0; 3]);
    }

    #[test]
    fn spawn_requires_a_walkable_face_or_manual_override() {
        let empty = ColFile::new(Vec::new(), Vec::new());
        let error = BlankStagePreset::default()
            .build(empty_model(), empty.clone(), bootstrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains("choose a spawn manually"), "{error}");

        let preset = BlankStagePreset {
            spawn: Some(JDramaTransform {
                translation: [10.0, 20.0, 30.0],
                rotation: [0.0, 90.0, 0.0],
                scale: [2.0; 3],
            }),
            ..BlankStagePreset::default()
        };
        assert!(preset.build(empty_model(), empty, bootstrap()).is_ok());

        let mut death_plane = floor_collision();
        death_plane.groups_mut()[0].surface_type = 0x800;
        assert!(BlankStagePreset::default()
            .build(empty_model(), death_plane, bootstrap())
            .is_err());
    }

    #[test]
    fn bootstrap_manifest_is_complete_typed_and_source_detached() {
        let model = empty_model().to_bytes().unwrap();
        let collision = floor_collision().encode().unwrap();
        let mut source =
            BLANK_STAGE_BOOTSTRAP_REQUIREMENTS.map(|requirement| BlankStageBootstrapResource {
                raw_path: requirement.raw_path.to_vec(),
                bytes: match requirement.kind {
                    BlankStageBootstrapKind::Model => model.clone(),
                    BlankStageBootstrapKind::Collision => collision.clone(),
                },
            });
        let manifest = BlankStageBootstrapManifest::from_authored_bytes(source.clone()).unwrap();
        for resource in &mut source {
            resource.bytes.fill(0xA5);
        }
        let archive = BlankStagePreset::default()
            .build(empty_model(), floor_collision(), manifest)
            .unwrap();
        let encoded = archive.encode().unwrap();
        assert_eq!(
            SourceFreeStageArchive::parse(&encoded)
                .unwrap()
                .encode()
                .unwrap(),
            encoded
        );

        let missing = BLANK_STAGE_BOOTSTRAP_REQUIREMENTS[..4]
            .iter()
            .map(|requirement| BlankStageBootstrapResource {
                raw_path: requirement.raw_path.to_vec(),
                bytes: match requirement.kind {
                    BlankStageBootstrapKind::Model => model.clone(),
                    BlankStageBootstrapKind::Collision => collision.clone(),
                },
            });
        assert!(BlankStageBootstrapManifest::from_authored_bytes(missing).is_err());
    }

    #[test]
    fn test11_target_metadata_is_external_and_patch_free() {
        let metadata = BlankStagePreset::default().target_metadata().unwrap();
        assert_eq!(metadata.target_slot, "test11");
        assert_eq!(metadata.output_archive_name, "test11.szs");
        assert!(metadata.replaces_existing_stage_mapping);
        assert!(!metadata.runtime_patch_required);
    }
}
