//! Applies editor-authored semantic changes to the strict stage archive and
//! writes a rebuilt archive outside the extracted base game.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sms_formats::{
    discover_scene_archives, ColFile, J3dRebuildDocument, JDramaRecord, JDramaRecordPayload,
    JDramaTransform,
};

use crate::{
    PlacementAddress, PlacementBinding, Result, SceneError, SceneObject, SourceFreeStageArchive,
    StageDocument, StageObjectPlacement, StageResourceDocument,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageArchiveEdits {
    #[serde(default)]
    pub models: Vec<StageModelEdit>,
    #[serde(default)]
    pub collisions: Vec<StageCollisionEdit>,
    #[serde(default)]
    pub placement_inserts: Vec<StagePlacementInsert>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageModelEdit {
    pub raw_resource_path: Vec<u8>,
    pub document: J3dRebuildDocument,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageCollisionEdit {
    pub raw_resource_path: Vec<u8>,
    pub document: ColFile,
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
    pub fn replace_model(
        &mut self,
        raw_resource_path: impl Into<Vec<u8>>,
        document: J3dRebuildDocument,
    ) {
        let raw_resource_path = raw_resource_path.into();
        if let Some(edit) = self
            .models
            .iter_mut()
            .find(|edit| edit.raw_resource_path == raw_resource_path)
        {
            edit.document = document;
        } else {
            self.models.push(StageModelEdit {
                raw_resource_path,
                document,
            });
        }
    }

    pub fn replace_collision(&mut self, raw_resource_path: impl Into<Vec<u8>>, document: ColFile) {
        let raw_resource_path = raw_resource_path.into();
        if let Some(edit) = self
            .collisions
            .iter_mut()
            .find(|edit| edit.raw_resource_path == raw_resource_path)
        {
            edit.document = document;
        } else {
            self.collisions.push(StageCollisionEdit {
                raw_resource_path,
                document,
            });
        }
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
            .models
            .iter()
            .map(|edit| edit.raw_resource_path.as_slice()),
        "model",
    )?;
    reject_duplicate_edit_paths(
        edits
            .collisions
            .iter()
            .map(|edit| edit.raw_resource_path.as_slice()),
        "collision",
    )?;
    for edit in &edits.models {
        let mut replacement = edit.document.clone();
        replacement
            .canonicalize_geometry_layout()
            .map_err(|source| SceneError::StageResource {
                path: display_raw_path(&edit.raw_resource_path),
                source,
            })?;
        match archive.resource_mut(&edit.raw_resource_path) {
            Some(StageResourceDocument::Model(document)) => *document = replacement,
            Some(_) => {
                return Err(stage_export_error(format!(
                    "{} is not a model resource",
                    display_raw_path(&edit.raw_resource_path)
                )))
            }
            None => {
                return Err(stage_export_error(format!(
                    "model resource {} was not found",
                    display_raw_path(&edit.raw_resource_path)
                )))
            }
        }
    }
    for edit in &edits.collisions {
        match archive.resource_mut(&edit.raw_resource_path) {
            Some(StageResourceDocument::Collision(document)) => *document = edit.document.clone(),
            Some(_) => {
                return Err(stage_export_error(format!(
                    "{} is not a collision resource",
                    display_raw_path(&edit.raw_resource_path)
                )))
            }
            None => {
                return Err(stage_export_error(format!(
                    "collision resource {} was not found",
                    display_raw_path(&edit.raw_resource_path)
                )))
            }
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
    for (address, object, mut record) in clones {
        let JDramaRecordPayload::Actor { transform, .. } = &mut record.payload else {
            return Err(stage_export_error(format!(
                "clone template for '{}' is not an actor",
                object.id
            )));
        };
        *transform = to_jdrama_transform(object);
        let (_, parent_path) = address.record_path.split_last().ok_or_else(|| {
            stage_export_error(format!("object '{}' references the root record", object.id))
        })?;
        archive.insert_placement_record(&address.raw_resource_path, parent_path, record)?;
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
        ColGroup, ColTriangle, ColVertex, JDramaDocument, JDramaLightMap, RarcDocument,
        RarcEntryRecord, RarcLayout, RarcNodeRecord,
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
