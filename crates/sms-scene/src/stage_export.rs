//! Applies editor-authored semantic changes to the strict stage archive and
//! writes a rebuilt archive outside the extracted base game.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sms_formats::{
    discover_scene_archives, ColFile, J3dRebuildDocument, JDramaDocument, JDramaField,
    JDramaFieldValue, JDramaRecord, JDramaRecordPayload, JDramaTransform,
};

use crate::{
    PlacementAddress, PlacementBinding, Result, SceneError, SceneObject, SourceFreeStageArchive,
    StageDocument, StageObjectPlacement, StageResourceDocument,
};

const WORLD_COLLISION_PATH: &[u8] = b"map/map.col";
const WORLD_SCENE_PATH: &[u8] = b"map/scene.bin";
const COLLISION_GRID_WIDTH_FIELD: &str = "collision_grid_width";
const COLLISION_GRID_HEIGHT_FIELD: &str = "collision_grid_height";
const COLLISION_TRIANGLE_CAPACITY_FIELD: &str = "collision_triangle_capacity";
const COLLISION_LIST_CAPACITY_FIELD: &str = "collision_list_capacity";
const COLLISION_WARP_CAPACITY_FIELD: &str = "collision_warp_capacity";
const COLLISION_GRID_CELL_SIZE: f32 = 1024.0;
const COLLISION_GRID_CELL_RECIPROCAL: f32 = 1.0 / COLLISION_GRID_CELL_SIZE;
const COLLISION_WALL_PADDING: f32 = 80.0;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageArchiveEdits {
    #[serde(default)]
    pub resources: Vec<StageResourceEdit>,
    #[serde(default)]
    pub models: Vec<StageModelEdit>,
    #[serde(default)]
    pub collisions: Vec<StageCollisionEdit>,
    #[serde(default)]
    pub placement_inserts: Vec<StagePlacementInsert>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageResourceEdit {
    pub raw_resource_path: Vec<u8>,
    pub document: StageResourceDocument,
    #[serde(default)]
    pub mode: StageResourceEditMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageResourceEditMode {
    #[default]
    Insert,
    Upsert,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageModelEdit {
    pub raw_resource_path: Vec<u8>,
    pub document: J3dRebuildDocument,
    #[serde(default)]
    pub mode: StageModelEditMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageModelEditMode {
    #[default]
    Replace,
    Upsert,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageCollisionEdit {
    pub raw_resource_path: Vec<u8>,
    pub document: ColFile,
    #[serde(default)]
    pub mode: StageCollisionEditMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageCollisionEditMode {
    #[default]
    Replace,
    Upsert,
    Append,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StagePlacementInsert {
    pub raw_resource_path: Vec<u8>,
    pub parent_record_path: Vec<usize>,
    pub record: JDramaRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageArchiveExportOutcome {
    pub source_path: PathBuf,
    pub output_path: PathBuf,
    pub size_bytes: usize,
    pub changed: bool,
}

impl StageArchiveEdits {
    pub fn insert_resource(
        &mut self,
        raw_resource_path: impl Into<Vec<u8>>,
        document: StageResourceDocument,
    ) {
        self.set_resource_edit(
            raw_resource_path.into(),
            document,
            StageResourceEditMode::Insert,
        );
    }

    pub fn upsert_resource(
        &mut self,
        raw_resource_path: impl Into<Vec<u8>>,
        document: StageResourceDocument,
    ) {
        self.set_resource_edit(
            raw_resource_path.into(),
            document,
            StageResourceEditMode::Upsert,
        );
    }

    fn set_resource_edit(
        &mut self,
        raw_resource_path: Vec<u8>,
        document: StageResourceDocument,
        mode: StageResourceEditMode,
    ) {
        if let Some(edit) = self
            .resources
            .iter_mut()
            .find(|edit| edit.raw_resource_path == raw_resource_path)
        {
            edit.document = document;
            edit.mode = mode;
        } else {
            self.resources.push(StageResourceEdit {
                raw_resource_path,
                document,
                mode,
            });
        }
    }

    pub fn replace_model(
        &mut self,
        raw_resource_path: impl Into<Vec<u8>>,
        document: J3dRebuildDocument,
    ) {
        self.set_model_edit(
            raw_resource_path.into(),
            document,
            StageModelEditMode::Replace,
        );
    }

    pub fn upsert_model(
        &mut self,
        raw_resource_path: impl Into<Vec<u8>>,
        document: J3dRebuildDocument,
    ) {
        self.set_model_edit(
            raw_resource_path.into(),
            document,
            StageModelEditMode::Upsert,
        );
    }

    fn set_model_edit(
        &mut self,
        raw_resource_path: Vec<u8>,
        document: J3dRebuildDocument,
        mode: StageModelEditMode,
    ) {
        if let Some(edit) = self
            .models
            .iter_mut()
            .find(|edit| edit.raw_resource_path == raw_resource_path)
        {
            edit.document = document;
            edit.mode = mode;
        } else {
            self.models.push(StageModelEdit {
                raw_resource_path,
                document,
                mode,
            });
        }
    }

    pub fn replace_collision(&mut self, raw_resource_path: impl Into<Vec<u8>>, document: ColFile) {
        self.set_collision_edit(
            raw_resource_path.into(),
            document,
            StageCollisionEditMode::Replace,
        );
    }

    pub fn upsert_collision(&mut self, raw_resource_path: impl Into<Vec<u8>>, document: ColFile) {
        self.set_collision_edit(
            raw_resource_path.into(),
            document,
            StageCollisionEditMode::Upsert,
        );
    }

    /// Appends a collision document after the current document at this path.
    /// Multiple append edits for one path are retained and applied in order.
    pub fn append_collision(&mut self, raw_resource_path: impl Into<Vec<u8>>, document: ColFile) {
        self.collisions.push(StageCollisionEdit {
            raw_resource_path: raw_resource_path.into(),
            document,
            mode: StageCollisionEditMode::Append,
        });
    }

    fn set_collision_edit(
        &mut self,
        raw_resource_path: Vec<u8>,
        document: ColFile,
        mode: StageCollisionEditMode,
    ) {
        // A replacement/upsert starts a new base for this path. Appends made
        // after it remain ordered, while earlier operations are superseded.
        self.collisions
            .retain(|edit| edit.raw_resource_path != raw_resource_path);
        self.collisions.push(StageCollisionEdit {
            raw_resource_path,
            document,
            mode,
        });
    }

    /// Appends a complete typed JDrama record beneath an existing semantic
    /// group path. The record is rebuilt from fields and never carries an
    /// imported record buffer or stored key-code metadata.
    pub fn insert_placement(
        &mut self,
        raw_resource_path: impl Into<Vec<u8>>,
        parent_record_path: impl Into<Vec<usize>>,
        record: JDramaRecord,
    ) {
        self.placement_inserts.push(StagePlacementInsert {
            raw_resource_path: raw_resource_path.into(),
            parent_record_path: parent_record_path.into(),
            record,
        });
    }
}

impl StageDocument {
    /// Rebuilds the currently open stage with object transforms, deletions,
    /// typed duplicates, and fully typed placement inserts applied. The
    /// returned bytes never consult an imported record or archive buffer as an
    /// export fallback.
    pub fn build_stage_archive(&self) -> Result<Vec<u8>> {
        self.build_stage_archive_with_edits(&self.archive_edits)
    }

    /// Also applies explicitly supplied model, collision, and placement
    /// documents.
    pub fn build_stage_archive_with_edits(&self, edits: &StageArchiveEdits) -> Result<Vec<u8>> {
        Ok(self.build_stage_archive_inner(edits)?.rebuilt)
    }

    /// Creates a new external archive. Existing outputs and every path inside
    /// the extracted base directory are rejected.
    pub fn export_stage_archive_new(
        &self,
        output_path: impl AsRef<Path>,
    ) -> Result<StageArchiveExportOutcome> {
        self.export_stage_archive_with_edits_new(output_path, &self.archive_edits)
    }

    pub fn export_stage_archive_with_edits_new(
        &self,
        output_path: impl AsRef<Path>,
        edits: &StageArchiveEdits,
    ) -> Result<StageArchiveExportOutcome> {
        let built = self.build_stage_archive_inner(edits)?;
        let output_path = checked_external_output(&self.base_root, output_path.as_ref())?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)?;
        output.write_all(&built.rebuilt)?;
        output.sync_all()?;
        Ok(StageArchiveExportOutcome {
            source_path: built.source_path,
            output_path,
            size_bytes: built.rebuilt.len(),
            changed: built.changed,
        })
    }

    fn build_stage_archive_inner(&self, edits: &StageArchiveEdits) -> Result<BuiltStageArchive> {
        self.validate_for_export()?;
        let source_path = self.stage_archive_source_path.clone().ok_or_else(|| {
            stage_export_error("the stage has no semantic archive source identity")
        })?;
        let mut archive = self.stage_archive.clone().ok_or_else(|| {
            stage_export_error(
                "the stage has no detached semantic archive; reopen it with a supported strict importer",
            )
        })?;

        // This is regenerated from the pristine semantic import, not read from
        // the retail source path. It is both the edit baseline and proof that
        // the retained document remains independently encodable.
        let baseline = archive.encode()?;

        apply_resource_edits(&mut archive, edits)?;
        let inserted_placement_roots = apply_placement_inserts(&mut archive, edits)?;
        reconcile_scene_objects(&mut archive, &self.objects, &inserted_placement_roots)?;
        let rebuilt = archive.encode()?;
        let reopened = SourceFreeStageArchive::parse(&rebuilt)?;
        if reopened.encode()? != rebuilt {
            return Err(stage_export_error(format!(
                "the edited semantic rebuild of '{}' was not stable after reimport",
                self.stage_id
            )));
        }
        let changed = rebuilt != baseline;
        Ok(BuiltStageArchive {
            source_path,
            rebuilt,
            changed,
        })
    }
}

struct BuiltStageArchive {
    source_path: PathBuf,
    rebuilt: Vec<u8>,
    changed: bool,
}

fn apply_resource_edits(
    archive: &mut SourceFreeStageArchive,
    edits: &StageArchiveEdits,
) -> Result<()> {
    reject_duplicate_edit_paths(
        edits
            .resources
            .iter()
            .map(|edit| edit.raw_resource_path.as_slice()),
        "resource",
    )?;
    reject_duplicate_edit_paths(
        edits
            .models
            .iter()
            .map(|edit| edit.raw_resource_path.as_slice()),
        "model",
    )?;
    reject_duplicate_collision_bases(&edits.collisions)?;

    // General resources are applied first so later typed edits can update or
    // append to a resource intentionally created by this transaction.
    for edit in &edits.resources {
        match edit.mode {
            StageResourceEditMode::Insert => {
                archive.insert_resource(edit.raw_resource_path.clone(), edit.document.clone())?
            }
            StageResourceEditMode::Upsert => {
                if archive.resource(&edit.raw_resource_path).is_some() {
                    archive.replace_resource(&edit.raw_resource_path, edit.document.clone())?;
                } else {
                    archive
                        .insert_resource(edit.raw_resource_path.clone(), edit.document.clone())?;
                }
            }
        }
    }

    for edit in &edits.models {
        let mut replacement = edit.document.clone();
        replacement
            .canonicalize_geometry_layout()
            .map_err(|source| SceneError::StageResource {
                path: display_raw_path(&edit.raw_resource_path),
                source,
            })?;
        match archive.resource(&edit.raw_resource_path) {
            Some(StageResourceDocument::Model(_)) => {
                archive.replace_resource(
                    &edit.raw_resource_path,
                    StageResourceDocument::Model(replacement),
                )?;
            }
            Some(_) => {
                return Err(stage_export_error(format!(
                    "{} is not a model resource",
                    display_raw_path(&edit.raw_resource_path)
                )));
            }
            None if edit.mode == StageModelEditMode::Upsert => {
                archive.insert_resource(
                    edit.raw_resource_path.clone(),
                    StageResourceDocument::Model(replacement),
                )?;
            }
            None => {
                return Err(stage_export_error(format!(
                    "model resource {} was not found",
                    display_raw_path(&edit.raw_resource_path)
                )));
            }
        }
    }
    for edit in &edits.collisions {
        match archive.resource(&edit.raw_resource_path) {
            Some(StageResourceDocument::Collision(document)) => {
                let replacement = match edit.mode {
                    StageCollisionEditMode::Replace | StageCollisionEditMode::Upsert => {
                        edit.document.clone()
                    }
                    StageCollisionEditMode::Append => append_collision_document(
                        document,
                        &edit.document,
                        &edit.raw_resource_path,
                    )?,
                };
                if edit.mode == StageCollisionEditMode::Append
                    && edit.raw_resource_path == WORLD_COLLISION_PATH
                {
                    preserve_world_collision_runtime_headroom(archive, &edit.document)?;
                }
                archive.replace_resource(
                    &edit.raw_resource_path,
                    StageResourceDocument::Collision(replacement),
                )?;
            }
            Some(_) => {
                return Err(stage_export_error(format!(
                    "{} is not a collision resource",
                    display_raw_path(&edit.raw_resource_path)
                )));
            }
            None if edit.mode == StageCollisionEditMode::Upsert => {
                archive.insert_resource(
                    edit.raw_resource_path.clone(),
                    StageResourceDocument::Collision(edit.document.clone()),
                )?;
            }
            None => {
                return Err(stage_export_error(format!(
                    "collision resource {} was not found",
                    display_raw_path(&edit.raw_resource_path)
                )));
            }
        }
    }
    Ok(())
}

fn append_collision_document(
    existing: &ColFile,
    authored: &ColFile,
    raw_resource_path: &[u8],
) -> Result<ColFile> {
    // Validate the authored document in its own index space before remapping.
    authored
        .encode()
        .map_err(|source| SceneError::StageResource {
            path: display_raw_path(raw_resource_path),
            source,
        })?;

    let vertex_base = existing.vertices().len();
    let mut appended_groups = authored.groups().to_vec();
    for (group_index, group) in appended_groups.iter_mut().enumerate() {
        for (triangle_index, triangle) in group.triangles.iter_mut().enumerate() {
            for vertex_index in &mut triangle.vertex_indices {
                let remapped = vertex_base
                    .checked_add(usize::from(*vertex_index))
                    .ok_or_else(|| {
                        stage_export_error(format!(
                            "collision append for {} overflowed while remapping group {group_index} triangle {triangle_index}",
                            display_raw_path(raw_resource_path)
                        ))
                    })?;
                if remapped > i16::MAX as usize {
                    return Err(stage_export_error(format!(
                        "collision append for {} cannot remap group {group_index} triangle {triangle_index} vertex {vertex_index}: index {remapped} exceeds the retail COL signed-index limit {}",
                        display_raw_path(raw_resource_path),
                        i16::MAX
                    )));
                }
                *vertex_index = remapped as u16;
            }
        }
    }

    let mut merged = existing.clone();
    merged.vertices_mut().extend_from_slice(authored.vertices());
    // Retail groups stay byte-semantically unchanged and in their original
    // order. Authored groups are appended rather than coalesced by surface.
    merged.groups_mut().append(&mut appended_groups);
    merged
        .encode()
        .map_err(|source| SceneError::StageResource {
            path: display_raw_path(raw_resource_path),
            source,
        })?;
    Ok(merged)
}

#[derive(Debug, Clone, Copy)]
struct MapCollisionRuntimeConfig {
    grid_width: i32,
    grid_height: i32,
    triangle_capacity: i32,
    list_capacity: i32,
}

#[derive(Debug, Clone, Copy)]
struct CollisionTrianglePoints {
    points: [[f32; 3]; 3],
    normal_y: f32,
}

fn preserve_world_collision_runtime_headroom(
    archive: &mut SourceFreeStageArchive,
    authored: &ColFile,
) -> Result<()> {
    let placement = match archive.resource_mut(WORLD_SCENE_PATH) {
        Some(StageResourceDocument::Placement(document)) => document,
        Some(_) => {
            return Err(stage_export_error(format!(
                "{} is not a typed placement resource required by {}",
                display_raw_path(WORLD_SCENE_PATH),
                display_raw_path(WORLD_COLLISION_PATH)
            )));
        }
        None => {
            return Err(stage_export_error(format!(
                "world collision append requires placement resource {}",
                display_raw_path(WORLD_SCENE_PATH)
            )));
        }
    };
    let map_record = unique_map_record_mut(placement)?;
    let JDramaRecordPayload::Fields { fields } = &mut map_record.payload else {
        return Err(stage_export_error(format!(
            "the unique Map record in {} does not have a typed fields payload",
            display_raw_path(WORLD_SCENE_PATH)
        )));
    };
    let config = read_map_collision_runtime_config(fields)?;
    let triangle_delta = authored_collision_triangle_count(authored)?;
    let list_delta = authored_collision_grid_link_count(authored, config)?;
    let triangle_capacity = config
        .triangle_capacity
        .checked_add(triangle_delta)
        .ok_or_else(|| {
            stage_export_error(format!(
                "Map field '{COLLISION_TRIANGLE_CAPACITY_FIELD}' overflows i32 while preserving {triangle_delta} authored collision triangles"
            ))
        })?;
    let list_capacity = config.list_capacity.checked_add(list_delta).ok_or_else(|| {
        stage_export_error(format!(
            "Map field '{COLLISION_LIST_CAPACITY_FIELD}' overflows i32 while preserving {list_delta} authored collision grid links"
        ))
    })?;

    set_unique_i32_field(fields, COLLISION_TRIANGLE_CAPACITY_FIELD, triangle_capacity)?;
    set_unique_i32_field(fields, COLLISION_LIST_CAPACITY_FIELD, list_capacity)?;
    Ok(())
}

fn unique_map_record_mut(document: &mut JDramaDocument) -> Result<&mut JDramaRecord> {
    let mut paths = Vec::new();
    collect_record_paths_by_type(&document.root, "Map", &mut Vec::new(), &mut paths);
    let path = match paths.as_slice() {
        [path] => path.as_slice(),
        [] => {
            return Err(stage_export_error(format!(
                "{} has no typed Map record for world collision capacities",
                display_raw_path(WORLD_SCENE_PATH)
            )));
        }
        _ => {
            return Err(stage_export_error(format!(
                "{} has {} typed Map records; world collision capacity target is ambiguous",
                display_raw_path(WORLD_SCENE_PATH),
                paths.len()
            )));
        }
    };
    jdrama_record_mut_at(&mut document.root, path)
}

fn collect_record_paths_by_type(
    record: &JDramaRecord,
    expected_type: &str,
    path: &mut Vec<usize>,
    matches: &mut Vec<Vec<usize>>,
) {
    if semantic_record_type_name(&record.type_name) == expected_type {
        matches.push(path.clone());
    }
    if let JDramaRecordPayload::Group { children, .. } = &record.payload {
        for (index, child) in children.iter().enumerate() {
            path.push(index);
            collect_record_paths_by_type(child, expected_type, path, matches);
            path.pop();
        }
    }
}

fn jdrama_record_mut_at<'a>(
    mut record: &'a mut JDramaRecord,
    path: &[usize],
) -> Result<&'a mut JDramaRecord> {
    for (depth, index) in path.iter().copied().enumerate() {
        let JDramaRecordPayload::Group { children, .. } = &mut record.payload else {
            return Err(stage_export_error(format!(
                "Map record path {} crosses a non-group at depth {depth}",
                display_record_path(path)
            )));
        };
        record = children.get_mut(index).ok_or_else(|| {
            stage_export_error(format!(
                "Map record path {} has no child {index} at depth {depth}",
                display_record_path(path)
            ))
        })?;
    }
    Ok(record)
}

fn semantic_record_type_name(type_name: &str) -> &str {
    type_name.rsplit("::").next().unwrap_or(type_name)
}

fn read_map_collision_runtime_config(fields: &[JDramaField]) -> Result<MapCollisionRuntimeConfig> {
    let grid_width = unique_i32_field(fields, COLLISION_GRID_WIDTH_FIELD)?;
    let grid_height = unique_i32_field(fields, COLLISION_GRID_HEIGHT_FIELD)?;
    let triangle_capacity = unique_i32_field(fields, COLLISION_TRIANGLE_CAPACITY_FIELD)?;
    let list_capacity = unique_i32_field(fields, COLLISION_LIST_CAPACITY_FIELD)?;
    let warp_capacity = unique_i32_field(fields, COLLISION_WARP_CAPACITY_FIELD)?;

    if grid_width <= 0 || grid_height <= 0 {
        return Err(stage_export_error(format!(
            "Map collision grid dimensions must be positive, got {grid_width}x{grid_height}"
        )));
    }
    grid_width.checked_mul(grid_height).ok_or_else(|| {
        stage_export_error(format!(
            "Map collision grid dimensions {grid_width}x{grid_height} overflow the runtime cell count"
        ))
    })?;
    for (name, value) in [
        (COLLISION_TRIANGLE_CAPACITY_FIELD, triangle_capacity),
        (COLLISION_LIST_CAPACITY_FIELD, list_capacity),
        (COLLISION_WARP_CAPACITY_FIELD, warp_capacity),
    ] {
        if value < 0 {
            return Err(stage_export_error(format!(
                "Map field '{name}' must be non-negative, got {value}"
            )));
        }
    }

    Ok(MapCollisionRuntimeConfig {
        grid_width,
        grid_height,
        triangle_capacity,
        list_capacity,
    })
}

fn unique_i32_field(fields: &[JDramaField], name: &str) -> Result<i32> {
    let index = unique_field_index(fields, name)?;
    match fields[index].value {
        JDramaFieldValue::I32(value) => Ok(value),
        _ => Err(stage_export_error(format!(
            "Map field '{name}' in {} is not typed i32",
            display_raw_path(WORLD_SCENE_PATH)
        ))),
    }
}

fn set_unique_i32_field(fields: &mut [JDramaField], name: &str, value: i32) -> Result<()> {
    let index = unique_field_index(fields, name)?;
    let JDramaFieldValue::I32(current) = &mut fields[index].value else {
        return Err(stage_export_error(format!(
            "Map field '{name}' in {} is not typed i32",
            display_raw_path(WORLD_SCENE_PATH)
        )));
    };
    *current = value;
    Ok(())
}

fn unique_field_index(fields: &[JDramaField], name: &str) -> Result<usize> {
    let matches = fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| (field.name == name).then_some(index))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => Err(stage_export_error(format!(
            "Map record in {} is missing field '{name}'",
            display_raw_path(WORLD_SCENE_PATH)
        ))),
        _ => Err(stage_export_error(format!(
            "Map record in {} has {} fields named '{name}'",
            display_raw_path(WORLD_SCENE_PATH),
            matches.len()
        ))),
    }
}

fn authored_collision_triangle_count(authored: &ColFile) -> Result<i32> {
    let count = authored.groups().iter().try_fold(0usize, |count, group| {
        count.checked_add(group.triangles.len()).ok_or_else(|| {
            stage_export_error("authored world collision triangle count overflowed usize")
        })
    })?;
    i32::try_from(count).map_err(|_| {
        stage_export_error(format!(
            "authored world collision has {count} triangles, exceeding the runtime i32 capacity"
        ))
    })
}

fn authored_collision_grid_link_count(
    authored: &ColFile,
    config: MapCollisionRuntimeConfig,
) -> Result<i32> {
    let extent_x = (config.grid_width / 2) as f32 * COLLISION_GRID_CELL_SIZE;
    let extent_z = (config.grid_height / 2) as f32 * COLLISION_GRID_CELL_SIZE;
    if !extent_x.is_finite() || !extent_z.is_finite() {
        return Err(stage_export_error(
            "Map collision grid extents are not finite",
        ));
    }

    let mut link_count = 0i32;
    for (group_index, group) in authored.groups().iter().enumerate() {
        for (triangle_index, triangle) in group.triangles.iter().enumerate() {
            let points = collision_triangle_points(
                authored,
                triangle.vertex_indices,
                group_index,
                triangle_index,
            )?;
            let plane_type = collision_plane_type(group.surface_type, points.normal_y);
            let Some([min_x, min_z, max_x, max_z]) = collision_grid_bounds(
                points.points,
                plane_type,
                extent_x,
                extent_z,
                config.grid_width,
                config.grid_height,
                group_index,
                triangle_index,
            )?
            else {
                continue;
            };

            for z_index in min_z..=max_z {
                for x_index in min_x..=max_x {
                    let cell_min_x = x_index as f32 * COLLISION_GRID_CELL_SIZE - extent_x;
                    let cell_min_z = z_index as f32 * COLLISION_GRID_CELL_SIZE - extent_z;
                    let cell_max_x = (x_index + 1) as f32 * COLLISION_GRID_CELL_SIZE - extent_x;
                    let cell_max_z = (z_index + 1) as f32 * COLLISION_GRID_CELL_SIZE - extent_z;
                    let (cell_min_x, cell_min_z, cell_max_x, cell_max_z) =
                        if plane_type == CollisionPlaneType::Wall {
                            (
                                cell_min_x - COLLISION_WALL_PADDING,
                                cell_min_z - COLLISION_WALL_PADDING,
                                cell_max_x + COLLISION_WALL_PADDING,
                                cell_max_z + COLLISION_WALL_PADDING,
                            )
                        } else {
                            (cell_min_x, cell_min_z, cell_max_x, cell_max_z)
                        };
                    if polygon_is_in_grid(cell_min_x, cell_min_z, cell_max_x, cell_max_z, points) {
                        link_count = link_count.checked_add(1).ok_or_else(|| {
                            stage_export_error(format!(
                                "authored world collision grid-link count exceeds i32 at group {group_index} triangle {triangle_index}"
                            ))
                        })?;
                    }
                }
            }
        }
    }
    Ok(link_count)
}

fn collision_triangle_points(
    authored: &ColFile,
    indices: [u16; 3],
    group_index: usize,
    triangle_index: usize,
) -> Result<CollisionTrianglePoints> {
    let mut points = [[0.0; 3]; 3];
    for (point, index) in points.iter_mut().zip(indices) {
        *point = authored
            .vertices()
            .get(usize::from(index))
            .ok_or_else(|| {
                stage_export_error(format!(
                    "authored world collision group {group_index} triangle {triangle_index} references missing vertex {index}"
                ))
            })?
            .position;
        if point.iter().any(|component| !component.is_finite()) {
            return Err(stage_export_error(format!(
                "authored world collision group {group_index} triangle {triangle_index} has a non-finite vertex"
            )));
        }
    }

    let [point_1, point_2, point_3] = points;
    let normal = [
        (point_2[1] - point_1[1]) * (point_3[2] - point_2[2])
            - (point_2[2] - point_1[2]) * (point_3[1] - point_2[1]),
        (point_2[2] - point_1[2]) * (point_3[0] - point_2[0])
            - (point_2[0] - point_1[0]) * (point_3[2] - point_2[2]),
        (point_2[0] - point_1[0]) * (point_3[1] - point_2[1])
            - (point_2[1] - point_1[1]) * (point_3[0] - point_2[0]),
    ];
    if normal.iter().any(|component| !component.is_finite()) {
        return Err(stage_export_error(format!(
            "authored world collision group {group_index} triangle {triangle_index} overflows while calculating its normal"
        )));
    }
    let normal_y = if normal.iter().any(|component| *component != 0.0) {
        let magnitude_squared =
            normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
        if !magnitude_squared.is_finite() || magnitude_squared <= 0.0 {
            return Err(stage_export_error(format!(
                "authored world collision group {group_index} triangle {triangle_index} has an unrepresentable normal"
            )));
        }
        normal[1] / magnitude_squared.sqrt()
    } else {
        0.0
    };
    if !normal_y.is_finite() {
        return Err(stage_export_error(format!(
            "authored world collision group {group_index} triangle {triangle_index} has a non-finite normalized normal"
        )));
    }
    Ok(CollisionTrianglePoints { points, normal_y })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollisionPlaneType {
    Ground,
    Roof,
    Wall,
}

fn collision_plane_type(surface_type: u16, normal_y: f32) -> CollisionPlaneType {
    if surface_type == 0x0801 || normal_y > 0.2 {
        CollisionPlaneType::Ground
    } else if normal_y < -0.2 {
        CollisionPlaneType::Roof
    } else {
        CollisionPlaneType::Wall
    }
}

#[allow(clippy::too_many_arguments)]
fn collision_grid_bounds(
    points: [[f32; 3]; 3],
    plane_type: CollisionPlaneType,
    extent_x: f32,
    extent_z: f32,
    grid_width: i32,
    grid_height: i32,
    group_index: usize,
    triangle_index: usize,
) -> Result<Option<[i32; 4]>> {
    let mut min_x = points[0][0].min(points[1][0]).min(points[2][0]);
    let mut min_z = points[0][2].min(points[1][2]).min(points[2][2]);
    let mut max_x = points[0][0].max(points[1][0]).max(points[2][0]);
    let mut max_z = points[0][2].max(points[1][2]).max(points[2][2]);
    if max_x < -extent_x || max_z < -extent_z || min_x > extent_x || min_z > extent_z {
        return Ok(None);
    }
    if plane_type == CollisionPlaneType::Wall {
        min_x -= COLLISION_WALL_PADDING;
        min_z -= COLLISION_WALL_PADDING;
        max_x += COLLISION_WALL_PADDING;
        max_z += COLLISION_WALL_PADDING;
    }
    if [min_x, min_z, max_x, max_z]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(stage_export_error(format!(
            "authored world collision group {group_index} triangle {triangle_index} has non-finite grid bounds"
        )));
    }

    let min_x = checked_trunc_grid_index(
        (min_x + extent_x) * COLLISION_GRID_CELL_RECIPROCAL,
        group_index,
        triangle_index,
    )?
    .max(0);
    let min_z = checked_trunc_grid_index(
        (min_z + extent_z) * COLLISION_GRID_CELL_RECIPROCAL,
        group_index,
        triangle_index,
    )?
    .max(0);
    let max_x = checked_trunc_grid_index(
        (max_x + extent_x) * COLLISION_GRID_CELL_RECIPROCAL,
        group_index,
        triangle_index,
    )?
    .min(grid_width - 1);
    let max_z = checked_trunc_grid_index(
        (max_z + extent_z) * COLLISION_GRID_CELL_RECIPROCAL,
        group_index,
        triangle_index,
    )?
    .min(grid_height - 1);
    if min_x > max_x || min_z > max_z {
        return Ok(None);
    }
    Ok(Some([min_x, min_z, max_x, max_z]))
}

fn checked_trunc_grid_index(value: f32, group_index: usize, triangle_index: usize) -> Result<i32> {
    if !value.is_finite() || (value as f64) < i32::MIN as f64 || (value as f64) > i32::MAX as f64 {
        return Err(stage_export_error(format!(
            "authored world collision group {group_index} triangle {triangle_index} has a grid index outside i32"
        )));
    }
    Ok(value as i32)
}

fn polygon_is_in_grid(
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
    triangle: CollisionTrianglePoints,
) -> bool {
    if triangle.normal_y < 0.0 {
        return true;
    }
    if triangle
        .points
        .iter()
        .any(|point| point_is_in_grid(point[0], point[2], min_x, min_z, max_x, max_z))
    {
        return true;
    }
    if [
        (min_x, min_z),
        (max_x, min_z),
        (min_x, max_z),
        (max_x, max_z),
    ]
    .into_iter()
    .any(|(x, z)| point_is_in_polygon(x, z, triangle.points))
    {
        return true;
    }
    check_line_polygon_collision(min_x, min_z, max_x, min_z, triangle.points)
        || check_line_polygon_collision(min_x, max_z, max_x, max_z, triangle.points)
        || check_line_polygon_collision(min_x, min_z, min_x, max_z, triangle.points)
        || check_line_polygon_collision(max_x, min_z, max_x, max_z, triangle.points)
}

fn point_is_in_grid(x: f32, z: f32, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> bool {
    min_x <= x && x <= max_x && min_z <= z && z <= max_z
}

fn point_is_in_polygon(x: f32, z: f32, points: [[f32; 3]; 3]) -> bool {
    let [point_1, point_2, point_3] = points;
    if (point_1[2] - z) * (point_2[0] - point_1[0]) - (point_1[0] - x) * (point_2[2] - point_1[2])
        < 0.0
    {
        return false;
    }
    if (point_2[2] - z) * (point_3[0] - point_2[0]) - (point_2[0] - x) * (point_3[2] - point_2[2])
        < 0.0
    {
        return false;
    }
    (point_3[2] - z) * (point_1[0] - point_3[0]) - (point_3[0] - x) * (point_1[2] - point_3[2])
        >= 0.0
}

fn check_line_polygon_collision(
    start_x: f32,
    start_z: f32,
    end_x: f32,
    end_z: f32,
    points: [[f32; 3]; 3],
) -> bool {
    let [point_1, point_2, point_3] = points;
    check_lines_collision(
        start_x, start_z, end_x, end_z, point_1[0], point_1[2], point_2[0], point_2[2],
    ) || check_lines_collision(
        start_x, start_z, end_x, end_z, point_2[0], point_2[2], point_3[0], point_3[2],
    ) || check_lines_collision(
        start_x, start_z, end_x, end_z, point_3[0], point_3[2], point_1[0], point_1[2],
    )
}

#[allow(clippy::too_many_arguments)]
fn check_lines_collision(
    a_x: f32,
    a_z: f32,
    b_x: f32,
    b_z: f32,
    c_x: f32,
    c_z: f32,
    d_x: f32,
    d_z: f32,
) -> bool {
    let delta_ab_x = b_x - a_x;
    let delta_ab_z = b_z - a_z;
    let cross_c = delta_ab_z * (c_x - b_x) - delta_ab_x * (c_z - b_z);
    let cross_d = delta_ab_z * (d_x - b_x) - delta_ab_x * (d_z - b_z);
    if (cross_c >= 0.0 && cross_d >= 0.0) || (cross_c < 0.0 && cross_d < 0.0) {
        return false;
    }

    let delta_cd_x = d_x - c_x;
    let delta_cd_z = d_z - c_z;
    let cross_a = delta_cd_z * (a_x - d_x) - delta_cd_x * (a_z - d_z);
    let cross_b = delta_cd_z * (b_x - d_x) - delta_cd_x * (b_z - d_z);
    !((cross_a >= 0.0 && cross_b >= 0.0) || (cross_a < 0.0 && cross_b < 0.0))
}

fn reject_duplicate_collision_bases(edits: &[StageCollisionEdit]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for edit in edits {
        if edit.mode != StageCollisionEditMode::Append
            && !unique.insert(edit.raw_resource_path.clone())
        {
            return Err(stage_export_error(format!(
                "duplicate collision base edit for {}",
                display_raw_path(&edit.raw_resource_path)
            )));
        }
    }
    Ok(())
}

fn reject_duplicate_edit_paths<'a>(
    paths: impl Iterator<Item = &'a [u8]>,
    kind: &str,
) -> Result<()> {
    let mut unique = BTreeSet::new();
    for path in paths {
        if !unique.insert(path.to_vec()) {
            return Err(stage_export_error(format!(
                "duplicate {kind} edit for {}",
                display_raw_path(path)
            )));
        }
    }
    Ok(())
}

fn apply_placement_inserts(
    archive: &mut SourceFreeStageArchive,
    edits: &StageArchiveEdits,
) -> Result<Vec<PlacementAddress>> {
    let mut inserted_roots = Vec::with_capacity(edits.placement_inserts.len());
    for insert in &edits.placement_inserts {
        let record_path = archive
            .insert_placement_record(
                &insert.raw_resource_path,
                &insert.parent_record_path,
                insert.record.clone(),
            )
            .map_err(|error| {
                stage_export_error(format!(
                    "could not insert typed placement under {}:{}: {error}",
                    display_raw_path(&insert.raw_resource_path),
                    display_record_path(&insert.parent_record_path)
                ))
            })?;
        inserted_roots.push(PlacementAddress {
            raw_resource_path: insert.raw_resource_path.clone(),
            record_path,
        });
    }
    Ok(inserted_roots)
}

fn reconcile_scene_objects(
    archive: &mut SourceFreeStageArchive,
    objects: &[SceneObject],
    inserted_placement_roots: &[PlacementAddress],
) -> Result<()> {
    let baseline = archive
        .object_placements()
        .into_iter()
        .filter(|placement| is_editor_placement_resource(&placement.raw_resource_path))
        .filter(|placement| {
            !inserted_placement_roots.iter().any(|root| {
                root.raw_resource_path == placement.raw_resource_path
                    && placement.record_path.starts_with(&root.record_path)
            })
        })
        .map(|placement| {
            (
                PlacementAddress {
                    raw_resource_path: placement.raw_resource_path.clone(),
                    record_path: placement.record_path.clone(),
                },
                placement,
            )
        })
        .collect::<BTreeMap<_, _>>();
    if baseline.is_empty() && !objects.is_empty() {
        return Err(stage_export_error(
            "the archive has no canonical map/scene.bin actor records",
        ));
    }

    let mut existing = BTreeMap::<PlacementAddress, &SceneObject>::new();
    let mut clones = Vec::<(PlacementAddress, &SceneObject, JDramaRecord)>::new();
    for object in objects {
        let dirty_params = object
            .raw_params
            .iter()
            .filter(|(_, value)| value.is_dirty())
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        if !dirty_params.is_empty() {
            return Err(stage_export_error(format!(
                "object '{}' has unmodeled parameter edit(s): {}",
                object.id,
                dirty_params.join(", ")
            )));
        }
        let Some(binding) = object.placement.as_ref() else {
            return Err(stage_export_error(format!(
                "object '{}' has no typed JDrama placement constructor",
                object.id
            )));
        };
        let address = binding.address();
        let Some(placement) = baseline.get(address) else {
            return Err(stage_export_error(format!(
                "object '{}' references missing placement {}:{}",
                object.id,
                display_raw_path(&address.raw_resource_path),
                display_record_path(&address.record_path)
            )));
        };
        validate_object_identity(object, placement)?;
        match binding {
            PlacementBinding::Existing(address) => {
                if existing.insert(address.clone(), object).is_some() {
                    return Err(stage_export_error(format!(
                        "multiple existing objects reference {}:{}",
                        display_raw_path(&address.raw_resource_path),
                        display_record_path(&address.record_path)
                    )));
                }
            }
            PlacementBinding::CloneOf(address) => {
                clones.push((
                    address.clone(),
                    object,
                    placement_record(archive, address)?.clone(),
                ));
            }
        }
    }

    for (address, object) in &existing {
        archive.set_object_transform(
            &address.raw_resource_path,
            &address.record_path,
            to_jdrama_transform(object),
        )?;
    }

    // Appending clones does not shift any imported child index, so original
    // addresses remain valid for the deletions that follow.
    for (address, object, record) in clones {
        let (_, parent_path) = address.record_path.split_last().ok_or_else(|| {
            stage_export_error(format!("object '{}' references the root record", object.id))
        })?;
        let record_path =
            archive.insert_placement_record(&address.raw_resource_path, parent_path, record)?;
        archive.set_object_transform(
            &address.raw_resource_path,
            &record_path,
            to_jdrama_transform(object),
        )?;
    }

    let mut deletions = baseline
        .keys()
        .filter(|address| !existing.contains_key(*address))
        .cloned()
        .collect::<Vec<_>>();
    deletions.sort_by(|left, right| {
        right
            .record_path
            .len()
            .cmp(&left.record_path.len())
            .then_with(|| right.raw_resource_path.cmp(&left.raw_resource_path))
            .then_with(|| right.record_path.cmp(&left.record_path))
    });
    for address in deletions {
        archive.remove_placement_record(&address.raw_resource_path, &address.record_path)?;
    }
    Ok(())
}

fn validate_object_identity(object: &SceneObject, placement: &StageObjectPlacement) -> Result<()> {
    if object.factory_name != placement.type_name {
        return Err(stage_export_error(format!(
            "object '{}' changed factory from '{}' to '{}' without a typed field mapping",
            object.id, placement.type_name, object.factory_name
        )));
    }
    if let Some(name) = object.raw_param("name") {
        if name != placement.name {
            return Err(stage_export_error(format!(
                "object '{}' changed its JDrama name without a typed field mapping",
                object.id
            )));
        }
    }
    Ok(())
}

fn placement_record<'a>(
    archive: &'a SourceFreeStageArchive,
    address: &PlacementAddress,
) -> Result<&'a JDramaRecord> {
    let Some(StageResourceDocument::Placement(document)) =
        archive.resource(&address.raw_resource_path)
    else {
        return Err(stage_export_error(format!(
            "placement resource {} was not found",
            display_raw_path(&address.raw_resource_path)
        )));
    };
    let mut record = &document.root;
    for index in &address.record_path {
        let JDramaRecordPayload::Group { children, .. } = &record.payload else {
            return Err(stage_export_error(format!(
                "placement path {} crosses a non-group",
                display_record_path(&address.record_path)
            )));
        };
        record = children.get(*index).ok_or_else(|| {
            stage_export_error(format!(
                "placement path {} is outside {}",
                display_record_path(&address.record_path),
                display_raw_path(&address.raw_resource_path)
            ))
        })?;
    }
    Ok(record)
}

fn to_jdrama_transform(object: &SceneObject) -> JDramaTransform {
    JDramaTransform {
        translation: object.transform.translation,
        rotation: object.transform.rotation_degrees,
        scale: object.transform.scale,
    }
}

fn is_editor_placement_resource(raw_path: &[u8]) -> bool {
    let lower = raw_path
        .iter()
        .map(|byte| byte.to_ascii_lowercase())
        .collect::<Vec<_>>();
    lower == b"scene.bin" || lower == b"map/scene.bin" || lower.ends_with(b"/map/scene.bin")
}

fn exact_stage_archive_path(base_root: &Path, stage_id: &str) -> Result<PathBuf> {
    let matches = discover_scene_archives(base_root)?
        .into_iter()
        .filter(|archive| archive.stage_id.eq_ignore_ascii_case(stage_id))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [archive] => Ok(archive.path.clone()),
        [] => Err(stage_export_error(format!(
            "no scene archive exactly matches stage '{stage_id}'"
        ))),
        _ => Err(stage_export_error(format!(
            "multiple scene archives exactly match stage '{stage_id}'"
        ))),
    }
}

/// Imports the complete strict semantic archive once. The source bytes are
/// used only for this import proof and are dropped before the document enters
/// editor state or project persistence.
pub(crate) fn import_exact_stage_archive(
    base_root: &Path,
    stage_id: &str,
) -> Result<(PathBuf, SourceFreeStageArchive)> {
    let source_path = exact_stage_archive_path(base_root, stage_id)?;
    let source = fs::read(&source_path)?;
    let archive = SourceFreeStageArchive::parse(&source)?;
    let rebuilt = archive.encode()?;
    if rebuilt != source {
        return Err(stage_export_error(format!(
            "the unedited semantic rebuild of '{stage_id}' was not byte-identical"
        )));
    }
    Ok((source_path, archive))
}

fn checked_external_output(base_root: &Path, output_path: &Path) -> Result<PathBuf> {
    let parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| stage_export_error("output path must include an existing parent"))?;
    if !parent.is_dir() {
        return Err(stage_export_error(format!(
            "output parent does not exist: {}",
            parent.display()
        )));
    }
    let file_name = output_path
        .file_name()
        .ok_or_else(|| stage_export_error("output path has no archive filename"))?;
    let canonical_base = fs::canonicalize(base_root)?;
    let canonical_output = fs::canonicalize(parent)?.join(file_name);
    if path_is_same_or_child(&canonical_output, &canonical_base) {
        return Err(SceneError::StageArchiveOutputOverlapsBase(canonical_output));
    }
    Ok(canonical_output)
}

fn path_is_same_or_child(path: &Path, parent: &Path) -> bool {
    let normalize = |value: &Path| {
        value
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase()
    };
    let path = normalize(path);
    let parent = normalize(parent);
    path == parent
        || path
            .strip_prefix(&parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn display_raw_path(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

fn display_record_path(path: &[usize]) -> String {
    if path.is_empty() {
        return "root".to_string();
    }
    path.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

fn stage_export_error(message: impl Into<String>) -> SceneError {
    SceneError::StageExport(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    use sms_formats::{
        ColGroup, ColTriangle, ColVertex, JDramaDocument, JDramaField, JDramaFieldValue,
        JDramaLightMap, PrmEntry, PrmFile, PrmValue, RarcDocument, RarcEntryRecord, RarcLayout,
        RarcNodeRecord,
    };

    use crate::{PlacementBinding, Transform};

    #[test]
    fn object_transform_delete_and_typed_clone_survive_reimport() {
        let fixture = StageFixture::new("object-edits");
        let mut document = fixture.document();
        document.objects[0].transform.translation = [10.0, 20.0, 30.0];
        document.objects.remove(1);
        let mut clone = document.objects[0].clone();
        clone.id = "clone".to_string();
        clone.placement = Some(PlacementBinding::CloneOf(address(&[0])));
        clone.transform.translation = [40.0, 50.0, 60.0];
        document.objects.push(clone);

        let rebuilt = document.build_stage_archive().unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        assert_eq!(reopened.encode().unwrap(), rebuilt);
        let placements = reopened.object_placements();
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].name, "first");
        assert_eq!(placements[0].transform.translation, [10.0, 20.0, 30.0]);
        assert_eq!(placements[1].name, "first");
        assert_eq!(placements[1].transform.translation, [40.0, 50.0, 60.0]);
    }

    #[test]
    fn typed_field_placement_transform_and_clone_survive_reimport() {
        let fixture = StageFixture::new("typed-field-placement");
        let mut document = fixture.document();
        let archive = document.stage_archive.as_mut().unwrap();
        let path = archive
            .insert_placement_record(b"scene.bin", &[], area_cylinder_record("area"))
            .unwrap();
        assert_eq!(path, [2]);

        let mut area = SceneObject::new("retail-area", "AreaCylinder");
        area.placement = Some(PlacementBinding::Existing(address(&path)));
        area.insert_source_raw_param("name", "area");
        area.transform = Transform {
            translation: [100.0, 200.0, 300.0],
            rotation_degrees: [10.0, 20.0, 30.0],
            scale: [40.0, 50.0, 60.0],
        };
        document.objects.push(area.clone());

        let mut clone = area;
        clone.id = "area-clone".to_string();
        clone.placement = Some(PlacementBinding::CloneOf(address(&path)));
        clone.transform.translation = [400.0, 500.0, 600.0];
        document.objects.push(clone);

        let rebuilt = document.build_stage_archive().unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        assert_eq!(reopened.encode().unwrap(), rebuilt);
        let areas = reopened
            .object_placements()
            .into_iter()
            .filter(|placement| placement.type_name == "AreaCylinder")
            .collect::<Vec<_>>();
        assert_eq!(areas.len(), 2);
        assert_eq!(areas[0].transform.translation, [100.0, 200.0, 300.0]);
        assert_eq!(areas[0].transform.rotation, [10.0, 20.0, 30.0]);
        assert_eq!(areas[0].transform.scale, [40.0, 50.0, 60.0]);
        assert_eq!(areas[1].transform.translation, [400.0, 500.0, 600.0]);
    }

    #[test]
    fn genuinely_missing_placement_address_is_still_rejected() {
        let fixture = StageFixture::new("missing-placement-address");
        let mut document = fixture.document();
        document.objects[0].placement = Some(PlacementBinding::Existing(address(&[99])));

        let error = document.build_stage_archive().unwrap_err().to_string();
        assert!(
            error.contains("references missing placement scene.bin:99"),
            "{error}"
        );
    }

    #[test]
    fn source_less_and_dirty_unmodeled_objects_are_rejected() {
        let fixture = StageFixture::new("reject-untyped");
        let mut source_less = fixture.document();
        source_less
            .objects
            .push(SceneObject::new("spawned", "MapStaticObj"));
        let error = source_less.build_stage_archive().unwrap_err().to_string();
        assert!(
            error.contains("no typed JDrama placement constructor"),
            "{error}"
        );

        let mut dirty = fixture.document();
        dirty.objects[0].set_raw_param("resource_name", "edited");
        let error = dirty.build_stage_archive().unwrap_err().to_string();
        assert!(error.contains("unmodeled parameter edit"), "{error}");
    }

    #[test]
    fn typed_model_and_collision_edits_reach_the_rebuilt_archive() {
        let fixture = StageFixture::new("resource-edits");
        let document = fixture.document();
        let source = fs::read(&fixture.archive_path).unwrap();
        let source_archive = SourceFreeStageArchive::parse(&source).unwrap();
        let mut model = match source_archive.resource(b"map.bmd").unwrap() {
            StageResourceDocument::Model(model) => model.clone(),
            _ => panic!("fixture model has wrong kind"),
        };
        model.reserved_words[0] = 0x1234_5678;
        let mut collision = match source_archive.resource(b"map.col").unwrap() {
            StageResourceDocument::Collision(collision) => collision.clone(),
            _ => panic!("fixture collision has wrong kind"),
        };
        collision.vertices_mut()[0].position[1] = 75.0;
        let mut edits = StageArchiveEdits::default();
        edits.replace_model(b"map.bmd".to_vec(), model);
        edits.replace_collision(b"map.col".to_vec(), collision);

        let rebuilt = document.build_stage_archive_with_edits(&edits).unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        let StageResourceDocument::Model(model) = reopened.resource(b"map.bmd").unwrap() else {
            panic!("rebuilt model has wrong kind");
        };
        assert_eq!(model.reserved_words[0], 0x1234_5678);
        let StageResourceDocument::Collision(collision) = reopened.resource(b"map.col").unwrap()
        else {
            panic!("rebuilt collision has wrong kind");
        };
        assert_eq!(collision.vertices()[0].position[1], 75.0);
    }

    #[test]
    fn resource_and_typed_upserts_insert_missing_archive_children() {
        let fixture = StageFixture::new("resource-upserts");
        let document = fixture.document();
        let source = fs::read(&fixture.archive_path).unwrap();
        let source_archive = SourceFreeStageArchive::parse(&source).unwrap();
        let mut model = match source_archive.resource(b"map.bmd").unwrap() {
            StageResourceDocument::Model(model) => model.clone(),
            _ => panic!("fixture model has wrong kind"),
        };
        model.reserved_words[0] = 0xAABB_CCDD;
        let collision = authored_collision(0x4100, 20.0, 3);
        let parameters = PrmFile {
            entries: vec![PrmEntry {
                name: "mSize".to_string(),
                value: PrmValue::from_f32(2.5),
            }],
        };
        let mut edits = StageArchiveEdits::default();
        edits.insert_resource(
            b"mapobj/authored.prm".to_vec(),
            StageResourceDocument::Parameters(parameters.clone()),
        );
        edits.upsert_model(b"mapobj/authored.bmd".to_vec(), model.clone());
        edits.upsert_collision(b"mapobj/authored.col".to_vec(), collision.clone());

        let rebuilt = document.build_stage_archive_with_edits(&edits).unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        assert_eq!(
            reopened.resource(b"mapobj/authored.prm"),
            Some(&StageResourceDocument::Parameters(parameters))
        );
        assert_eq!(
            reopened.resource(b"mapobj/authored.bmd"),
            Some(&StageResourceDocument::Model(model))
        );
        assert_eq!(
            reopened.resource(b"mapobj/authored.col"),
            Some(&StageResourceDocument::Collision(collision))
        );
        assert_eq!(reopened.encode().unwrap(), rebuilt);
    }

    #[test]
    fn general_insert_rejects_existing_paths_while_explicit_upsert_replaces() {
        let fixture = StageFixture::new("general-resource-modes");
        let document = fixture.document();
        let collision = authored_collision(0x4100, 12.0, 4);
        let mut insert = StageArchiveEdits::default();
        insert.insert_resource(
            b"map.col".to_vec(),
            StageResourceDocument::Collision(collision.clone()),
        );
        let error = document
            .build_stage_archive_with_edits(&insert)
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"), "{error}");

        let mut upsert = StageArchiveEdits::default();
        upsert.upsert_resource(
            b"map.col".to_vec(),
            StageResourceDocument::Collision(collision.clone()),
        );
        let rebuilt = document.build_stage_archive_with_edits(&upsert).unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        assert_eq!(
            reopened.resource(b"map.col"),
            Some(&StageResourceDocument::Collision(collision))
        );
    }

    #[test]
    fn replace_model_and_collision_remain_replacement_only() {
        let fixture = StageFixture::new("replacement-only");
        let document = fixture.document();
        let source = fs::read(&fixture.archive_path).unwrap();
        let source_archive = SourceFreeStageArchive::parse(&source).unwrap();
        let model = match source_archive.resource(b"map.bmd").unwrap() {
            StageResourceDocument::Model(model) => model.clone(),
            _ => panic!("fixture model has wrong kind"),
        };
        let mut model_edits = StageArchiveEdits::default();
        model_edits.replace_model(b"missing.bmd".to_vec(), model);
        let error = document
            .build_stage_archive_with_edits(&model_edits)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("model resource missing.bmd was not found"),
            "{error}"
        );

        let mut collision_edits = StageArchiveEdits::default();
        collision_edits.replace_collision(b"missing.col".to_vec(), authored_collision(0, 0.0, 0));
        let error = document
            .build_stage_archive_with_edits(&collision_edits)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("collision resource missing.col was not found"),
            "{error}"
        );
    }

    #[test]
    fn collision_appends_preserve_retail_groups_and_remap_each_authored_index() {
        let fixture = StageFixture::new("collision-appends");
        let document = fixture.document();
        let source = fs::read(&fixture.archive_path).unwrap();
        let source_archive = SourceFreeStageArchive::parse(&source).unwrap();
        let original = match source_archive.resource(b"map.col").unwrap() {
            StageResourceDocument::Collision(collision) => collision.clone(),
            _ => panic!("fixture collision has wrong kind"),
        };
        let first = authored_collision(0x4100, 20.0, 3);
        let second = authored_collision(0x4200, 40.0, 7);
        let mut edits = StageArchiveEdits::default();
        edits.append_collision(b"map.col".to_vec(), first.clone());
        edits.append_collision(b"map.col".to_vec(), second.clone());

        let rebuilt = document.build_stage_archive_with_edits(&edits).unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        let collision = match reopened.resource(b"map.col").unwrap() {
            StageResourceDocument::Collision(collision) => collision,
            _ => panic!("rebuilt collision has wrong kind"),
        };
        assert_eq!(
            &collision.vertices()[..original.vertices().len()],
            original.vertices()
        );
        assert_eq!(
            &collision.groups()[..original.groups().len()],
            original.groups()
        );
        assert_eq!(collision.vertices().len(), 9);
        assert_eq!(collision.groups().len(), 3);
        assert_eq!(
            collision.groups()[1].surface_type,
            first.groups()[0].surface_type
        );
        assert_eq!(collision.groups()[1].triangles[0].vertex_indices, [3, 4, 5]);
        assert_eq!(
            collision.groups()[2].surface_type,
            second.groups()[0].surface_type
        );
        assert_eq!(collision.groups()[2].triangles[0].vertex_indices, [6, 7, 8]);
    }

    #[test]
    fn world_collision_append_preserves_duck_like_runtime_headroom() {
        let fixture = StageFixture::new("world-collision-capacities");
        let mut document = fixture.document();
        install_world_collision_resources(&mut document);
        let authored = duck_like_collision();
        let config = MapCollisionRuntimeConfig {
            grid_width: 60,
            grid_height: 60,
            triangle_capacity: 12_000,
            list_capacity: 30_000,
        };
        assert_eq!(authored_collision_triangle_count(&authored).unwrap(), 4_212);
        assert_eq!(
            authored_collision_grid_link_count(&authored, config).unwrap(),
            4_668
        );

        let mut edits = StageArchiveEdits::default();
        edits.append_collision(WORLD_COLLISION_PATH.to_vec(), authored);
        let rebuilt = document.build_stage_archive_with_edits(&edits).unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        assert_eq!(
            world_map_collision_fields(&reopened),
            [60, 60, 16_212, 34_668, 3_000]
        );
        let StageResourceDocument::Collision(collision) =
            reopened.resource(WORLD_COLLISION_PATH).unwrap()
        else {
            panic!("rebuilt world collision has wrong kind");
        };
        assert_eq!(
            collision
                .groups()
                .iter()
                .map(|group| group.triangles.len())
                .sum::<usize>(),
            4_213
        );
        assert_eq!(reopened.encode().unwrap(), rebuilt);
    }

    #[test]
    fn collision_grid_links_apply_wall_padding_to_bounds_and_cells() {
        let authored = collision_with_triangle(
            0,
            [
                [1_100.0, 0.0, 100.0],
                [1_100.0, 100.0, 100.0],
                [1_100.0, 0.0, 200.0],
            ],
        );
        let config = MapCollisionRuntimeConfig {
            grid_width: 4,
            grid_height: 4,
            triangle_capacity: 0,
            list_capacity: 0,
        };
        let points = collision_triangle_points(&authored, [0, 1, 2], 0, 0).unwrap();

        assert_eq!(
            collision_plane_type(authored.groups()[0].surface_type, points.normal_y),
            CollisionPlaneType::Wall
        );
        assert_eq!(
            collision_grid_bounds(
                points.points,
                CollisionPlaneType::Wall,
                2_048.0,
                2_048.0,
                4,
                4,
                0,
                0,
            )
            .unwrap(),
            Some([2, 2, 3, 2])
        );
        assert!(!polygon_is_in_grid(0.0, 0.0, 1_024.0, 1_024.0, points));
        assert!(polygon_is_in_grid(
            -COLLISION_WALL_PADDING,
            -COLLISION_WALL_PADDING,
            1_024.0 + COLLISION_WALL_PADDING,
            1_024.0 + COLLISION_WALL_PADDING,
            points,
        ));
        assert_eq!(
            authored_collision_grid_link_count(&authored, config).unwrap(),
            2
        );
    }

    #[test]
    fn collision_grid_links_negative_normal_roofs_across_the_full_bbox() {
        let authored = collision_with_triangle(
            0,
            [
                [-500.0, 0.0, -500.0],
                [500.0, 0.0, -500.0],
                [-500.0, 0.0, 500.0],
            ],
        );
        let config = MapCollisionRuntimeConfig {
            grid_width: 4,
            grid_height: 4,
            triangle_capacity: 0,
            list_capacity: 0,
        };
        let points = collision_triangle_points(&authored, [0, 1, 2], 0, 0).unwrap();

        assert!(points.normal_y < 0.0);
        assert_eq!(
            collision_plane_type(authored.groups()[0].surface_type, points.normal_y),
            CollisionPlaneType::Roof
        );
        assert_eq!(
            collision_grid_bounds(
                points.points,
                CollisionPlaneType::Roof,
                2_048.0,
                2_048.0,
                4,
                4,
                0,
                0,
            )
            .unwrap(),
            Some([1, 1, 2, 2])
        );
        assert!(polygon_is_in_grid(0.0, 0.0, 1_024.0, 1_024.0, points));
        assert_eq!(
            authored_collision_grid_link_count(&authored, config).unwrap(),
            4
        );
    }

    #[test]
    fn collision_surface_0801_overrides_wall_classification_to_ground() {
        let points = [
            [1_100.0, 0.0, 100.0],
            [1_100.0, 100.0, 100.0],
            [1_100.0, 0.0, 200.0],
        ];
        let wall = collision_with_triangle(0, points);
        let forced_ground = collision_with_triangle(0x0801, points);
        let config = MapCollisionRuntimeConfig {
            grid_width: 4,
            grid_height: 4,
            triangle_capacity: 0,
            list_capacity: 0,
        };
        let triangle = collision_triangle_points(&forced_ground, [0, 1, 2], 0, 0).unwrap();

        assert_eq!(triangle.normal_y, 0.0);
        assert_eq!(
            collision_plane_type(0x0801, triangle.normal_y),
            CollisionPlaneType::Ground
        );
        assert_eq!(
            authored_collision_grid_link_count(&wall, config).unwrap(),
            2
        );
        assert_eq!(
            authored_collision_grid_link_count(&forced_ground, config).unwrap(),
            1
        );
    }

    #[test]
    fn collision_grid_bounds_clip_partial_triangles_and_reject_outside_ones() {
        let partial = collision_with_triangle(
            0,
            [
                [1_900.0, 0.0, 100.0],
                [1_900.0, 0.0, 300.0],
                [2_300.0, 0.0, 100.0],
            ],
        );
        let outside = collision_with_triangle(
            0,
            [
                [2_050.0, 0.0, 100.0],
                [2_050.0, 0.0, 300.0],
                [2_300.0, 0.0, 100.0],
            ],
        );
        let config = MapCollisionRuntimeConfig {
            grid_width: 4,
            grid_height: 4,
            triangle_capacity: 0,
            list_capacity: 0,
        };
        let partial_points = collision_triangle_points(&partial, [0, 1, 2], 0, 0).unwrap();
        let outside_points = collision_triangle_points(&outside, [0, 1, 2], 0, 0).unwrap();

        assert_eq!(
            collision_grid_bounds(
                partial_points.points,
                CollisionPlaneType::Ground,
                2_048.0,
                2_048.0,
                4,
                4,
                0,
                0,
            )
            .unwrap(),
            Some([3, 2, 3, 2])
        );
        assert_eq!(
            collision_grid_bounds(
                outside_points.points,
                CollisionPlaneType::Ground,
                2_048.0,
                2_048.0,
                4,
                4,
                0,
                0,
            )
            .unwrap(),
            None
        );
        assert_eq!(
            authored_collision_grid_link_count(&partial, config).unwrap(),
            1
        );
        assert_eq!(
            authored_collision_grid_link_count(&outside, config).unwrap(),
            0
        );
    }

    #[test]
    fn collision_grid_links_match_odd_runtime_extents_and_truncation() {
        let boundary = collision_with_triangle(
            0,
            [
                [2_048.0, 0.0, 0.0],
                [2_048.0, 0.0, 10.0],
                [2_050.0, 0.0, 0.0],
            ],
        );
        let outside = collision_with_triangle(
            0,
            [
                [2_048.25, 0.0, 0.0],
                [2_048.25, 0.0, 10.0],
                [2_050.0, 0.0, 0.0],
            ],
        );
        let config = MapCollisionRuntimeConfig {
            grid_width: 5,
            grid_height: 3,
            triangle_capacity: 0,
            list_capacity: 0,
        };
        let boundary_points = collision_triangle_points(&boundary, [0, 1, 2], 0, 0).unwrap();

        assert_eq!(
            collision_grid_bounds(
                boundary_points.points,
                CollisionPlaneType::Ground,
                2_048.0,
                1_024.0,
                5,
                3,
                0,
                0,
            )
            .unwrap(),
            Some([4, 1, 4, 1])
        );
        assert_eq!(checked_trunc_grid_index(-0.75, 0, 0).unwrap(), 0);
        assert_eq!(checked_trunc_grid_index(-1.75, 0, 0).unwrap(), -1);
        assert_eq!(
            authored_collision_grid_link_count(&boundary, config).unwrap(),
            1
        );
        assert_eq!(
            authored_collision_grid_link_count(&outside, config).unwrap(),
            0
        );
    }

    #[test]
    fn stale_and_non_world_collision_appends_do_not_change_map_capacities() {
        let fixture = StageFixture::new("non-world-collision-capacities");
        let mut document = fixture.document();
        install_world_collision_resources(&mut document);
        document
            .stage_archive
            .as_mut()
            .unwrap()
            .insert_resource(
                b"mapobj/authored.col".to_vec(),
                StageResourceDocument::Collision(authored_collision(0, 200.0, 1)),
            )
            .unwrap();

        let mut edits = StageArchiveEdits::default();
        edits.append_collision(b"map.col".to_vec(), authored_collision(0, 300.0, 2));
        edits.append_collision(
            b"mapobj/authored.col".to_vec(),
            authored_collision(0, 400.0, 3),
        );
        let rebuilt = document.build_stage_archive_with_edits(&edits).unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        assert_eq!(
            world_map_collision_fields(&reopened),
            [60, 60, 12_000, 30_000, 3_000]
        );
    }

    #[test]
    fn world_collision_capacity_target_rejects_missing_ambiguous_and_wrong_fields() {
        let authored = authored_collision(0, 100.0, 0);

        let mut missing = world_collision_archive("missing-map-field");
        world_map_fields_mut(&mut missing)
            .retain(|field| field.name != COLLISION_LIST_CAPACITY_FIELD);
        let error = preserve_world_collision_runtime_headroom(&mut missing, &authored)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("missing field 'collision_list_capacity'"),
            "{error}"
        );

        let mut ambiguous = world_collision_archive("ambiguous-map-record");
        let placement = match ambiguous.resource_mut(WORLD_SCENE_PATH).unwrap() {
            StageResourceDocument::Placement(document) => document,
            _ => panic!("world scene has wrong kind"),
        };
        let JDramaRecordPayload::Group { children, .. } = &mut placement.root.payload else {
            panic!("world scene root is not a group");
        };
        children.push(children[0].clone());
        let error = preserve_world_collision_runtime_headroom(&mut ambiguous, &authored)
            .unwrap_err()
            .to_string();
        assert!(error.contains("capacity target is ambiguous"), "{error}");

        let mut wrong_type = world_collision_archive("wrong-map-field-type");
        let fields = world_map_fields_mut(&mut wrong_type);
        let field = fields
            .iter_mut()
            .find(|field| field.name == COLLISION_TRIANGLE_CAPACITY_FIELD)
            .unwrap();
        field.value = JDramaFieldValue::U32(12_000);
        let error = preserve_world_collision_runtime_headroom(&mut wrong_type, &authored)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("collision_triangle_capacity") && error.contains("not typed i32"),
            "{error}"
        );
    }

    #[test]
    fn multiple_world_collision_appends_accumulate_capacity_deltas() {
        let fixture = StageFixture::new("multiple-world-collision-appends");
        let mut document = fixture.document();
        install_world_collision_resources(&mut document);
        let mut edits = StageArchiveEdits::default();
        edits.append_collision(
            WORLD_COLLISION_PATH.to_vec(),
            authored_collision(0, 100.0, 1),
        );
        edits.append_collision(
            WORLD_COLLISION_PATH.to_vec(),
            authored_collision(0, 200.0, 2),
        );

        let rebuilt = document.build_stage_archive_with_edits(&edits).unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        assert_eq!(
            world_map_collision_fields(&reopened),
            [60, 60, 12_002, 30_002, 3_000]
        );
    }

    #[test]
    fn world_collision_capacity_overflow_is_rejected_atomically() {
        let authored = authored_collision(0, 100.0, 0);

        let mut triangle_overflow = world_collision_archive("triangle-capacity-overflow");
        set_unique_i32_field(
            world_map_fields_mut(&mut triangle_overflow),
            COLLISION_TRIANGLE_CAPACITY_FIELD,
            i32::MAX,
        )
        .unwrap();
        let before = world_map_collision_fields(&triangle_overflow);
        let error = preserve_world_collision_runtime_headroom(&mut triangle_overflow, &authored)
            .unwrap_err()
            .to_string();
        assert!(error.contains("collision_triangle_capacity"), "{error}");
        assert_eq!(world_map_collision_fields(&triangle_overflow), before);

        let mut list_overflow = world_collision_archive("list-capacity-overflow");
        set_unique_i32_field(
            world_map_fields_mut(&mut list_overflow),
            COLLISION_LIST_CAPACITY_FIELD,
            i32::MAX,
        )
        .unwrap();
        let before = world_map_collision_fields(&list_overflow);
        let error = preserve_world_collision_runtime_headroom(&mut list_overflow, &authored)
            .unwrap_err()
            .to_string();
        assert!(error.contains("collision_list_capacity"), "{error}");
        assert_eq!(world_map_collision_fields(&list_overflow), before);
    }

    #[test]
    fn collision_append_rejects_retail_signed_index_overflow() {
        let existing = ColFile::new(
            vec![ColVertex::new(0.0, 0.0, 0.0); i16::MAX as usize],
            Vec::new(),
        );
        let authored = ColFile::new(
            vec![ColVertex::new(1.0, 0.0, 0.0), ColVertex::new(2.0, 0.0, 0.0)],
            vec![ColGroup {
                surface_type: 0,
                has_per_triangle_data: false,
                triangles: vec![ColTriangle {
                    vertex_indices: [0, 1, 1],
                    attribute_0: 0,
                    attribute_1: 0,
                    data: None,
                }],
            }],
        );
        let error = append_collision_document(&existing, &authored, b"map.col")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("index 32768 exceeds the retail COL signed-index limit 32767"),
            "{error}"
        );
    }

    #[test]
    fn collision_append_accepts_retail_signed_index_maximum() {
        let existing = ColFile::new(
            vec![ColVertex::new(0.0, 0.0, 0.0); i16::MAX as usize],
            Vec::new(),
        );
        let authored = ColFile::new(
            vec![ColVertex::new(1.0, 0.0, 0.0)],
            vec![ColGroup {
                surface_type: 0,
                has_per_triangle_data: false,
                triangles: vec![ColTriangle {
                    vertex_indices: [0, 0, 0],
                    attribute_0: 0,
                    attribute_1: 0,
                    data: None,
                }],
            }],
        );

        let merged = append_collision_document(&existing, &authored, b"map.col").unwrap();
        assert_eq!(
            merged.groups()[0].triangles[0].vertex_indices,
            [i16::MAX as u16; 3]
        );
    }

    #[test]
    fn fully_typed_placement_insert_survives_fresh_reimport() {
        let fixture = StageFixture::new("typed-placement-insert");
        let mut document = fixture.document();
        document.archive_edits.insert_placement(
            b"scene.bin".to_vec(),
            Vec::new(),
            actor_record("inserted", [70.0, 80.0, 90.0]),
        );

        let rebuilt = document.build_stage_archive().unwrap();
        let reopened = SourceFreeStageArchive::parse(&rebuilt).unwrap();
        assert_eq!(reopened.encode().unwrap(), rebuilt);
        let placements = reopened.object_placements();
        assert_eq!(placements.len(), 3);
        assert_eq!(placements[2].type_name, "MapStaticObj");
        assert_eq!(placements[2].name, "inserted");
        assert_eq!(placements[2].transform.translation, [70.0, 80.0, 90.0]);
    }

    #[test]
    fn external_export_is_create_new_and_rejects_the_base_tree() {
        let fixture = StageFixture::new("external-output");
        let document = fixture.document();
        let output_root = unique_root("external-output-target");
        fs::create_dir_all(&output_root).unwrap();
        let output = output_root.join("test.arc");
        let outcome = document.export_stage_archive_new(&output).unwrap();
        assert_eq!(
            outcome.output_path,
            fs::canonicalize(&output_root).unwrap().join("test.arc")
        );
        assert!(!outcome.changed);
        assert_eq!(
            fs::read(&output).unwrap(),
            fs::read(&fixture.archive_path).unwrap()
        );
        assert!(document.export_stage_archive_new(&output).is_err());

        let inside_base = fixture.root.join("inside.arc");
        let error = document.export_stage_archive_new(&inside_base).unwrap_err();
        assert!(matches!(
            error,
            SceneError::StageArchiveOutputOverlapsBase(_)
        ));
        fs::remove_dir_all(output_root).unwrap();
    }

    #[test]
    fn opened_stage_exports_after_the_source_archive_is_overwritten() {
        let fixture = StageFixture::new("detached-after-open");
        let mut document = StageDocument::open(&fixture.root, "test").unwrap();
        assert!(document.stage_archive.is_some());
        assert_eq!(
            document.stage_archive_source_path.as_deref(),
            Some(fixture.archive_path.as_path())
        );
        document.objects = vec![
            fixture_object("first", &[0]),
            fixture_object("second", &[1]),
        ];
        let expected = document.stage_archive.as_ref().unwrap().encode().unwrap();

        fs::write(&fixture.archive_path, b"destroyed source archive").unwrap();
        let output_root = unique_root("detached-after-open-output");
        fs::create_dir_all(&output_root).unwrap();
        let output = output_root.join("test.arc");
        let outcome = document.export_stage_archive_new(&output).unwrap();

        assert!(!outcome.changed);
        assert_eq!(fs::read(output).unwrap(), expected);
        fs::remove_dir_all(output_root).unwrap();
    }

    #[test]
    fn project_round_trip_keeps_the_freshly_imported_semantic_archive() {
        let fixture = StageFixture::new("semantic-project-round-trip");
        let project_root = unique_root("semantic-project");
        let mut saved = fixture.document();
        saved.objects[0].transform.translation = [101.0, 202.0, 303.0];
        let expected_rebuild = saved.build_stage_archive().unwrap();
        saved.save_project_folder(&project_root).unwrap();

        let mut reopened = fixture.document();
        let fresh_archive = reopened.stage_archive.clone();
        assert!(reopened.load_project_folder(&project_root).unwrap());
        assert_eq!(reopened.stage_archive, fresh_archive);
        assert_eq!(
            reopened.objects[0].transform.translation,
            [101.0, 202.0, 303.0]
        );
        assert_eq!(reopened.build_stage_archive().unwrap(), expected_rebuild);

        fs::remove_dir_all(project_root).unwrap();
    }

    struct StageFixture {
        root: PathBuf,
        archive_path: PathBuf,
    }

    impl StageFixture {
        fn new(label: &str) -> Self {
            let root = unique_root(label);
            let scene_root = root.join("files/data/scene");
            fs::create_dir_all(&scene_root).unwrap();
            let archive_path = scene_root.join("test.arc");
            fs::write(&archive_path, fixture_archive()).unwrap();
            Self { root, archive_path }
        }

        fn document(&self) -> StageDocument {
            let source = fs::read(&self.archive_path).expect("read stage fixture archive");
            let stage_archive = SourceFreeStageArchive::parse(&source)
                .expect("import stage fixture archive semantically");
            StageDocument {
                stage_id: "test".to_string(),
                base_root: self.root.clone(),
                assets: Vec::new(),
                objects: vec![
                    fixture_object("first", &[0]),
                    fixture_object("second", &[1]),
                ],
                changed_files: BTreeMap::new(),
                stage_archive: Some(stage_archive),
                stage_archive_source_path: Some(self.archive_path.clone()),
                archive_edits: StageArchiveEdits::default(),
                registry: None,
                load_issues: Vec::new(),
                lighting: crate::StageLighting::default(),
                actor_previews: BTreeMap::new(),
                loaded_project: None,
            }
        }
    }

    impl Drop for StageFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn fixture_object(name: &str, path: &[usize]) -> SceneObject {
        let mut object = SceneObject::new(format!("retail-{name}"), "MapStaticObj");
        object.placement = Some(PlacementBinding::Existing(address(path)));
        object.transform = if path == [0] {
            Transform {
                translation: [1.0, 2.0, 3.0],
                rotation_degrees: [0.0, 90.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            }
        } else {
            Transform {
                translation: [4.0, 5.0, 6.0],
                rotation_degrees: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            }
        };
        object.insert_source_raw_param("name", name);
        object
    }

    fn address(path: &[usize]) -> PlacementAddress {
        PlacementAddress {
            raw_resource_path: b"scene.bin".to_vec(),
            record_path: path.to_vec(),
        }
    }

    fn install_world_collision_resources(document: &mut StageDocument) {
        let archive = document.stage_archive.as_mut().unwrap();
        archive
            .insert_resource(
                WORLD_SCENE_PATH.to_vec(),
                StageResourceDocument::Placement(world_map_scene_document()),
            )
            .unwrap();
        archive
            .insert_resource(
                WORLD_COLLISION_PATH.to_vec(),
                StageResourceDocument::Collision(authored_collision(0, 500.0, 0)),
            )
            .unwrap();
    }

    fn world_collision_archive(label: &str) -> SourceFreeStageArchive {
        let fixture = StageFixture::new(label);
        let mut document = fixture.document();
        install_world_collision_resources(&mut document);
        document.stage_archive.take().unwrap()
    }

    fn world_map_scene_document() -> JDramaDocument {
        JDramaDocument {
            root: JDramaRecord::new(
                "NameRefGrp",
                "root",
                JDramaRecordPayload::Group {
                    fields: Vec::new(),
                    children: vec![JDramaRecord::new(
                        "Map",
                        "map",
                        JDramaRecordPayload::Fields {
                            fields: vec![
                                JDramaField {
                                    name: "translucent_group_count".to_string(),
                                    value: JDramaFieldValue::U32(0),
                                },
                                JDramaField {
                                    name: COLLISION_GRID_WIDTH_FIELD.to_string(),
                                    value: JDramaFieldValue::I32(60),
                                },
                                JDramaField {
                                    name: COLLISION_GRID_HEIGHT_FIELD.to_string(),
                                    value: JDramaFieldValue::I32(60),
                                },
                                JDramaField {
                                    name: COLLISION_TRIANGLE_CAPACITY_FIELD.to_string(),
                                    value: JDramaFieldValue::I32(12_000),
                                },
                                JDramaField {
                                    name: COLLISION_LIST_CAPACITY_FIELD.to_string(),
                                    value: JDramaFieldValue::I32(30_000),
                                },
                                JDramaField {
                                    name: COLLISION_WARP_CAPACITY_FIELD.to_string(),
                                    value: JDramaFieldValue::I32(3_000),
                                },
                                JDramaField {
                                    name: "warp_pair_count".to_string(),
                                    value: JDramaFieldValue::U32(0),
                                },
                            ],
                        },
                    )
                    .unwrap()],
                },
            )
            .unwrap(),
        }
    }

    fn world_map_fields_mut(archive: &mut SourceFreeStageArchive) -> &mut Vec<JDramaField> {
        let placement = match archive.resource_mut(WORLD_SCENE_PATH).unwrap() {
            StageResourceDocument::Placement(document) => document,
            _ => panic!("world scene has wrong kind"),
        };
        let JDramaRecordPayload::Group { children, .. } = &mut placement.root.payload else {
            panic!("world scene root is not a group");
        };
        let JDramaRecordPayload::Fields { fields } = &mut children[0].payload else {
            panic!("Map record does not have fields");
        };
        fields
    }

    fn world_map_collision_fields(archive: &SourceFreeStageArchive) -> [i32; 5] {
        let placement = match archive.resource(WORLD_SCENE_PATH).unwrap() {
            StageResourceDocument::Placement(document) => document,
            _ => panic!("world scene has wrong kind"),
        };
        let JDramaRecordPayload::Group { children, .. } = &placement.root.payload else {
            panic!("world scene root is not a group");
        };
        let JDramaRecordPayload::Fields { fields } = &children[0].payload else {
            panic!("Map record does not have fields");
        };
        [
            unique_i32_field(fields, COLLISION_GRID_WIDTH_FIELD).unwrap(),
            unique_i32_field(fields, COLLISION_GRID_HEIGHT_FIELD).unwrap(),
            unique_i32_field(fields, COLLISION_TRIANGLE_CAPACITY_FIELD).unwrap(),
            unique_i32_field(fields, COLLISION_LIST_CAPACITY_FIELD).unwrap(),
            unique_i32_field(fields, COLLISION_WARP_CAPACITY_FIELD).unwrap(),
        ]
    }

    fn duck_like_collision() -> ColFile {
        let mut triangles = Vec::with_capacity(4_212);
        triangles.extend((0..3_756).map(|_| ColTriangle {
            vertex_indices: [0, 1, 2],
            attribute_0: 0,
            attribute_1: 0,
            data: None,
        }));
        triangles.extend((0..456).map(|_| ColTriangle {
            vertex_indices: [3, 4, 5],
            attribute_0: 0,
            attribute_1: 0,
            data: None,
        }));
        ColFile::new(
            vec![
                ColVertex::new(100.0, 0.0, 100.0),
                ColVertex::new(100.0, 0.0, 110.0),
                ColVertex::new(110.0, 0.0, 100.0),
                ColVertex::new(-10.0, 0.0, 100.0),
                ColVertex::new(-10.0, 0.0, 110.0),
                ColVertex::new(10.0, 0.0, 100.0),
            ],
            vec![ColGroup {
                surface_type: 0,
                has_per_triangle_data: false,
                triangles,
            }],
        )
    }

    fn collision_with_triangle(surface_type: u16, points: [[f32; 3]; 3]) -> ColFile {
        ColFile::new(
            points
                .into_iter()
                .map(|[x, y, z]| ColVertex::new(x, y, z))
                .collect(),
            vec![ColGroup {
                surface_type,
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

    fn authored_collision(surface_type: u16, x: f32, attribute: u8) -> ColFile {
        ColFile::new(
            vec![
                ColVertex::new(x, 0.0, 0.0),
                ColVertex::new(x + 1.0, 0.0, 0.0),
                ColVertex::new(x, 0.0, 1.0),
            ],
            vec![ColGroup {
                surface_type,
                has_per_triangle_data: false,
                triangles: vec![ColTriangle {
                    vertex_indices: [0, 1, 2],
                    attribute_0: attribute,
                    attribute_1: attribute.wrapping_add(1),
                    data: None,
                }],
            }],
        )
    }

    fn fixture_archive() -> Vec<u8> {
        let placement = JDramaDocument {
            root: JDramaRecord::new(
                "NameRefGrp",
                "root",
                JDramaRecordPayload::Group {
                    fields: Vec::new(),
                    children: vec![
                        actor_record("first", [1.0, 2.0, 3.0]),
                        actor_record("second", [4.0, 5.0, 6.0]),
                    ],
                },
            )
            .unwrap(),
        }
        .to_bytes()
        .unwrap();
        let collision = ColFile::new(
            vec![
                ColVertex::new(0.0, 0.0, 0.0),
                ColVertex::new(1.0, 0.0, 0.0),
                ColVertex::new(0.0, 0.0, 1.0),
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
        .encode()
        .unwrap();
        let model = J3dRebuildDocument {
            file_type: *b"bmd3",
            version_tag: *b"SVR3",
            reserved_words: [u32::MAX; 3],
            declared_section_count: 0,
            sections: Vec::new(),
        }
        .to_bytes()
        .unwrap();
        let resources = [
            (b"scene.bin".as_slice(), placement),
            (b"map.col".as_slice(), collision),
            (b"map.bmd".as_slice(), model),
        ];
        let mut entries = resources
            .into_iter()
            .enumerate()
            .map(|(index, (name, data))| RarcEntryRecord {
                file_id: index as u16,
                name_hash: 0,
                flags: 0x11,
                name_offset: 0,
                raw_name: name.to_vec(),
                data_offset: 0,
                size: data.len() as u32,
                reserved: 0,
                data: Some(data),
            })
            .collect::<Vec<_>>();
        entries.extend(root_directory_entries());
        let mut archive = RarcDocument {
            layout: RarcLayout {
                file_size: 0,
                header_size: 0x20,
                data_offset: 0,
                data_size: 0,
                mram_data_size: 0,
                aram_data_size: 0,
                dvd_data_size: 0,
                metadata_present: true,
                node_offset: 0,
                entry_offset: 0,
                string_table_offset: 0,
                string_table_size: 0,
                next_free_file_id: entries.len() as u16,
                sync_file_ids: 1,
                info_reserved: [0; 5],
                alignment: 0x20,
                padding_byte: 0,
            },
            nodes: vec![RarcNodeRecord {
                node_type: *b"ROOT",
                name_offset: 0,
                name_hash: 0,
                raw_name: b"root".to_vec(),
                entry_count: entries.len() as u16,
                first_entry_index: 0,
            }],
            entries,
        };
        archive.canonicalize_layout().unwrap();
        archive.to_bytes().unwrap()
    }

    fn root_directory_entries() -> [RarcEntryRecord; 2] {
        [
            RarcEntryRecord {
                file_id: u16::MAX,
                name_hash: 0,
                flags: 0x02,
                name_offset: 0,
                raw_name: b".".to_vec(),
                data_offset: 0,
                size: 0,
                reserved: 0,
                data: None,
            },
            RarcEntryRecord {
                file_id: u16::MAX,
                name_hash: 0,
                flags: 0x02,
                name_offset: 0,
                raw_name: b"..".to_vec(),
                data_offset: u32::MAX,
                size: 0,
                reserved: 0,
                data: None,
            },
        ]
    }

    fn actor_record(name: &str, translation: [f32; 3]) -> JDramaRecord {
        JDramaRecord::new(
            "MapStaticObj",
            name,
            JDramaRecordPayload::Actor {
                transform: JDramaTransform {
                    translation,
                    rotation: if name == "first" {
                        [0.0, 90.0, 0.0]
                    } else {
                        [0.0; 3]
                    },
                    scale: [1.0; 3],
                },
                character_name: name.to_string(),
                light_map: JDramaLightMap::default(),
                fields: Vec::new(),
            },
        )
        .unwrap()
    }

    fn area_cylinder_record(name: &str) -> JDramaRecord {
        JDramaRecord::new(
            "AreaCylinder",
            name,
            JDramaRecordPayload::Fields {
                fields: vec![
                    JDramaField {
                        name: "center".to_string(),
                        value: JDramaFieldValue::Vec3F32([1.0, 2.0, 3.0]),
                    },
                    JDramaField {
                        name: "authoring_vector".to_string(),
                        value: JDramaFieldValue::Vec3F32([4.0, 5.0, 6.0]),
                    },
                    JDramaField {
                        name: "cylinder_parameters".to_string(),
                        value: JDramaFieldValue::Vec3F32([7.0, 8.0, 9.0]),
                    },
                    JDramaField {
                        name: "authoring_character_name".to_string(),
                        value: JDramaFieldValue::String("area character".to_string()),
                    },
                    JDramaField {
                        name: "indexed_name_count".to_string(),
                        value: JDramaFieldValue::U32(0),
                    },
                    JDramaField {
                        name: "manager_group_name".to_string(),
                        value: JDramaFieldValue::String("area manager".to_string()),
                    },
                    JDramaField {
                        name: "raw_angle_hundredths".to_string(),
                        value: JDramaFieldValue::I32(0),
                    },
                ],
            },
        )
        .unwrap()
    }

    fn unique_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sms-stage-export-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
