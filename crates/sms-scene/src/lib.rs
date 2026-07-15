//! Editable stage documents and safe editor-project persistence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sms_formats::{
    parse_jdrama_object_records, read_stage_asset_bytes, scan_stage_assets, JDramaAmbient,
    JDramaLight, SourceLocation, StageAsset, StageAssetKind,
};
use sms_schema::{
    EnemyActorDefinition, EnemyManagerDefinition, EnemyModelDefinition, ObjectRegistry,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SceneError {
    #[error("base root does not exist: {0}")]
    MissingBaseRoot(PathBuf),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("format error: {0}")]
    Format(#[from] sms_formats::FormatError),
    #[error("manifest serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("scene overlay serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid stage id for an editor project path: {0}")]
    InvalidStageId(String),
    #[error("project output path must be relative and traversal-free: {0}")]
    UnsafeProjectPath(PathBuf),
    #[error("project output folder must have a parent and file name: {0}")]
    InvalidProjectRoot(PathBuf),
    #[error("project output folder overlaps the extracted base game directory: {0}")]
    ProjectOverlapsBase(PathBuf),
}

pub type Result<T> = std::result::Result<T, SceneError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorProjectManifest {
    pub format_version: u32,
    pub kind: String,
    pub base_path: PathBuf,
    pub project_files_path: PathBuf,
    pub created_with: String,
    pub changed_files: Vec<PathBuf>,
}

impl EditorProjectManifest {
    pub fn new(base_path: PathBuf, project_files_path: PathBuf) -> Self {
        Self {
            format_version: 1,
            kind: "sms-editor-project".to_string(),
            base_path,
            project_files_path,
            created_with: env!("CARGO_PKG_VERSION").to_string(),
            changed_files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageDocument {
    pub stage_id: String,
    pub base_root: PathBuf,
    pub assets: Vec<StageAsset>,
    pub objects: Vec<SceneObject>,
    pub changed_files: BTreeMap<PathBuf, Vec<u8>>,
    pub registry: Option<ObjectRegistry>,
    pub load_issues: Vec<ValidationIssue>,
    #[serde(default)]
    pub lighting: StageLighting,
    #[serde(skip)]
    pub actor_previews: BTreeMap<String, ActorPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorPreview {
    pub model_path: String,
    pub load_flags: u32,
    pub manager_factory: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageLighting {
    pub lights: Vec<JDramaLight>,
    pub ambients: Vec<JDramaAmbient>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageObjectLighting {
    pub position: [f32; 3],
    pub color: [u8; 4],
    pub ambient: [u8; 4],
}

impl StageLighting {
    pub fn object_lighting(&self) -> Option<StageObjectLighting> {
        let object_primary = |name: &str| {
            name.contains("オブジェクト")
                && name.contains("太陽")
                && !name.contains("サブ")
                && !name.contains("スペキュラ")
        };
        let light = self
            .lights
            .iter()
            .find(|light| light.name.as_deref().is_some_and(object_primary))
            .or_else(|| self.lights.get(5))?;
        let ambient = self
            .ambients
            .iter()
            .find(|ambient| {
                ambient.name.as_deref().is_some_and(|name| {
                    name.contains("オブジェクト")
                        && name.contains("アンビエント")
                        && !name.contains("サブ")
                })
            })
            .or_else(|| self.ambients.get(2))?;
        Some(StageObjectLighting {
            position: light.position,
            color: light.color,
            ambient: ambient.color,
        })
    }
}

impl StageDocument {
    pub fn open(base_root: impl AsRef<Path>, stage_id: impl Into<String>) -> Result<Self> {
        let base_root = base_root.as_ref().to_path_buf();
        if !base_root.exists() {
            return Err(SceneError::MissingBaseRoot(base_root));
        }

        let stage_id = stage_id.into();
        let assets = scan_stage_assets(&base_root, &stage_id)?;
        let (objects, load_issues, lighting) = load_scene_objects_from_assets(&assets);
        Ok(Self {
            stage_id,
            base_root,
            assets,
            objects,
            changed_files: BTreeMap::new(),
            registry: None,
            load_issues,
            lighting,
            actor_previews: BTreeMap::new(),
        })
    }

    pub fn with_registry(mut self, registry: ObjectRegistry) -> Self {
        self.set_registry(registry);
        self
    }

    pub fn set_registry(&mut self, registry: ObjectRegistry) {
        let (actor_previews, preview_issues) =
            build_actor_preview_catalog(&self.base_root, &self.assets, &registry);
        self.actor_previews = actor_previews;
        self.load_issues
            .retain(|issue| !issue.code.starts_with("enemy-preview-"));
        self.load_issues.extend(preview_issues);
        self.registry = Some(registry);
    }

    pub fn actor_preview(&self, object: &SceneObject) -> Option<&ActorPreview> {
        object
            .source
            .as_ref()
            .and_then(actor_preview_source_key)
            .and_then(|key| self.actor_previews.get(&key))
            .or_else(|| {
                self.actor_previews
                    .get(&actor_preview_factory_key(&object.factory_name))
            })
    }

    pub fn add_object(&mut self, object: SceneObject) {
        self.objects.push(object);
    }

    pub fn mark_changed_file(
        &mut self,
        relative_path: impl Into<PathBuf>,
        bytes: Vec<u8>,
    ) -> Result<()> {
        let relative_path = relative_path.into();
        validate_project_relative_path(&relative_path)?;
        self.changed_files.insert(relative_path, bytes);
        Ok(())
    }

    pub fn queue_editor_overlay_change(&mut self) -> Result<()> {
        let path = self.editor_overlay_path()?;
        if self.objects.is_empty() {
            self.changed_files.remove(&path);
            return Ok(());
        }

        let overlay = EditorSceneOverlay {
            stage_id: self.stage_id.clone(),
            objects: self.objects.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&overlay)?;
        self.mark_changed_file(path, bytes)?;
        Ok(())
    }

    pub fn editor_overlay_path(&self) -> Result<PathBuf> {
        validate_stage_id(&self.stage_id)?;
        Ok(PathBuf::from("editor")
            .join("stages")
            .join(format!("{}.scene.json", self.stage_id)))
    }

    pub fn save_project_folder(
        &self,
        project_root: impl AsRef<Path>,
    ) -> Result<EditorProjectManifest> {
        let project_root = project_root.as_ref();
        if project_root
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(SceneError::InvalidProjectRoot(project_root.to_path_buf()));
        }
        let project_comparison = normalized_absolute_for_comparison(project_root)?;
        let base_comparison = normalized_absolute_for_comparison(&self.base_root)?;
        if path_is_same_or_child(&project_comparison, &base_comparison)
            || path_is_same_or_child(&base_comparison, &project_comparison)
        {
            return Err(SceneError::ProjectOverlapsBase(project_root.to_path_buf()));
        }
        let parent = project_root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = project_root
            .file_name()
            .ok_or_else(|| SceneError::InvalidProjectRoot(project_root.to_path_buf()))?
            .to_string_lossy();
        fs::create_dir_all(parent)?;

        let unique = std::process::id();
        let staging_root = parent.join(format!(".{name}.staging-{unique}"));
        let backup_root = parent.join(format!(".{name}.backup-{unique}"));
        remove_dir_if_exists(&staging_root)?;
        remove_dir_if_exists(&backup_root)?;

        let files_root = staging_root.join("files");
        fs::create_dir_all(&files_root)?;

        let mut changed_files = Vec::new();
        for (relative_path, bytes) in &self.changed_files {
            validate_project_relative_path(relative_path)?;
            let out_path = files_root.join(relative_path);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&out_path, bytes)?;
            changed_files.push(relative_path.clone());
        }

        changed_files.sort();
        let mut manifest =
            EditorProjectManifest::new(self.base_root.clone(), project_root.join("files"));
        manifest.changed_files = changed_files;

        let manifest_text = toml::to_string_pretty(&manifest)?;
        fs::write(staging_root.join("sms-project.toml"), manifest_text)?;

        if project_root.exists() {
            fs::rename(project_root, &backup_root)?;
        }
        if let Err(err) = fs::rename(&staging_root, project_root) {
            if backup_root.exists() {
                let _ = fs::rename(&backup_root, project_root);
            }
            return Err(SceneError::Io(err));
        }
        remove_dir_if_exists(&backup_root)?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = self.load_issues.clone();

        if !self.base_root.exists() {
            issues.push(ValidationIssue::error(
                "missing-base-root",
                format!("Base root does not exist: {}", self.base_root.display()),
            ));
        }

        if self.assets.is_empty() {
            issues.push(ValidationIssue::warning(
                "no-stage-assets",
                format!("No assets found for stage '{}'", self.stage_id),
            ));
        }

        if validate_stage_id(&self.stage_id).is_err() {
            issues.push(ValidationIssue::error(
                "invalid-stage-id",
                format!(
                    "Stage id '{}' is not safe for project output",
                    self.stage_id
                ),
            ));
        }

        for path in self.changed_files.keys() {
            if validate_project_relative_path(path).is_err() {
                issues.push(ValidationIssue::error(
                    "unsafe-project-path",
                    format!("Changed file path is unsafe: {}", path.display()),
                ));
            }
        }

        let mut object_ids = BTreeSet::new();
        for object in &self.objects {
            if !object_ids.insert(object.id.as_str()) {
                issues.push(ValidationIssue::error(
                    "duplicate-object-id",
                    format!("Object id '{}' is duplicated", object.id),
                ));
            }
            if object.factory_name.trim().is_empty() {
                issues.push(ValidationIssue::error(
                    "empty-factory-name",
                    format!("Object {} has no factory name", object.id),
                ));
            }

            if !object.transform.is_finite() {
                issues.push(ValidationIssue::error(
                    "invalid-transform",
                    format!("Object {} has a non-finite transform", object.id),
                ));
            }
            if object
                .transform
                .scale
                .iter()
                .any(|value| value.abs() <= f32::EPSILON)
            {
                issues.push(ValidationIssue::warning(
                    "zero-scale",
                    format!("Object {} has a non-invertible scale", object.id),
                ));
            }

            if let Some(registry) = &self.registry {
                if registry.find_object(&object.factory_name).is_none() && object.source.is_none() {
                    issues.push(ValidationIssue::warning(
                        "unknown-factory",
                        format!(
                            "Object '{}' is not in the generated registry",
                            object.factory_name
                        ),
                    ));
                }
            }
        }

        issues
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorSceneOverlay {
    pub stage_id: String,
    pub objects: Vec<SceneObject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneObject {
    pub id: String,
    pub source: Option<SourceLocation>,
    pub factory_name: String,
    pub class_name: Option<String>,
    pub transform: Transform,
    pub raw_params: BTreeMap<String, String>,
    pub decoded_params: BTreeMap<String, ParamValue>,
    pub asset_hints: Vec<AssetRef>,
}

impl SceneObject {
    pub fn new(id: impl Into<String>, factory_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: None,
            factory_name: factory_name.into(),
            class_name: None,
            transform: Transform::default(),
            raw_params: BTreeMap::new(),
            decoded_params: BTreeMap::new(),
            asset_hints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: [f32; 3],
    pub rotation_degrees: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0, 0.0, 0.0],
            rotation_degrees: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }
    }
}

impl Transform {
    pub fn is_finite(&self) -> bool {
        self.translation
            .iter()
            .chain(self.rotation_degrees.iter())
            .chain(self.scale.iter())
            .all(|value| value.is_finite())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Float(f32),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRef {
    pub path: String,
    pub role: AssetRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetRole {
    PreviewModel,
    InferredPreviewModel,
    Collision,
    Texture,
    Animation,
    Script,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub code: String,
    pub message: String,
}

impl ValidationIssue {
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Warning,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Error,
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

fn validate_stage_id(stage_id: &str) -> Result<()> {
    let valid = !stage_id.is_empty()
        && stage_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && stage_id != "."
        && stage_id != "..";
    if valid {
        Ok(())
    } else {
        Err(SceneError::InvalidStageId(stage_id.to_string()))
    }
}

fn validate_project_relative_path(path: &Path) -> Result<()> {
    let valid = !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if valid {
        Ok(())
    } else {
        Err(SceneError::UnsafeProjectPath(path.to_path_buf()))
    }
}

fn remove_dir_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(SceneError::Io(err)),
    }
}

fn normalized_absolute_for_comparison(path: &Path) -> Result<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let canonical = canonicalize_with_missing_tail(&absolute);
    let normalized = canonical
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    Ok(normalized
        .strip_prefix("\\\\?\\")
        .unwrap_or(&normalized)
        .to_string())
}

fn canonicalize_with_missing_tail(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut missing = Vec::new();

    loop {
        if let Ok(mut canonical) = fs::canonicalize(existing) {
            for component in missing.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }

        let Some(name) = existing.file_name() else {
            return path.to_path_buf();
        };
        missing.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            return path.to_path_buf();
        };
        existing = parent;
    }
}

fn path_is_same_or_child(path: &str, parent: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn load_scene_objects_from_assets(
    assets: &[StageAsset],
) -> (Vec<SceneObject>, Vec<ValidationIssue>, StageLighting) {
    let mut objects = Vec::new();
    let mut issues = Vec::new();
    let mut lighting = StageLighting::default();
    let model_index = stage_model_index(assets);
    let mut placement_files = 0usize;

    for asset in assets
        .iter()
        .filter(|asset| asset.kind == StageAssetKind::Placement)
    {
        let path_text = asset.path.to_string_lossy().replace('\\', "/");
        if !path_text.to_ascii_lowercase().ends_with("/map/scene.bin") {
            continue;
        }
        placement_files += 1;

        let bytes = match read_stage_asset_bytes(&asset.path) {
            Ok(bytes) => bytes,
            Err(err) => {
                issues.push(ValidationIssue::error(
                    "placement-read-failed",
                    format!("Could not read {}: {err}", asset.path.display()),
                ));
                continue;
            }
        };
        let records = match parse_jdrama_object_records(&bytes) {
            Ok(records) => records,
            Err(err) => {
                issues.push(ValidationIssue::error(
                    "placement-parse-failed",
                    format!("Could not parse {}: {err}", asset.path.display()),
                ));
                continue;
            }
        };

        for record in records {
            if let Some(light) = record.light.clone() {
                lighting.lights.push(light);
            }
            if let Some(ambient) = record.ambient.clone() {
                lighting.ambients.push(ambient);
            }
            let Some(transform) = record.transform else {
                continue;
            };

            let type_name = record.type_name.clone();
            let object_name = record
                .object_name
                .clone()
                .unwrap_or_else(|| type_name.clone());
            let mut object =
                SceneObject::new(format!("retail-{:08x}", record.offset), type_name.clone());
            object.source = Some(SourceLocation {
                path: asset.path.clone(),
                offset: Some(record.offset as u64),
                length: Some(record.size as u64),
            });
            object.class_name = Some(type_name);
            object.transform = Transform {
                translation: transform.translation,
                rotation_degrees: transform.rotation,
                scale: transform.scale,
            };
            object
                .raw_params
                .insert("name".to_string(), object_name.clone());
            for (index, value) in record.stream_strings.iter().enumerate() {
                object
                    .raw_params
                    .insert(format!("stream_string_{index}"), value.clone());
            }
            if let Some(params) = record.npc_params {
                object.raw_params.insert(
                    "npc_body_color_index".to_string(),
                    params.color_indices[0].to_string(),
                );
                object.raw_params.insert(
                    "npc_cloth_color_index".to_string(),
                    params.color_indices[1].to_string(),
                );
                object.raw_params.insert(
                    "npc_pollution_amount".to_string(),
                    params.pollution_amount.to_string(),
                );
                object
                    .raw_params
                    .insert("npc_parts_mask".to_string(), params.parts_mask.to_string());
                for (index, value) in params.parts_color_indices.into_iter().enumerate() {
                    object
                        .raw_params
                        .insert(format!("npc_parts_color_index_{index}"), value.to_string());
                }
                object.raw_params.insert(
                    "npc_action_flags".to_string(),
                    params.action_flags.to_string(),
                );
            }
            if let Some(blade_count) = record.map_obj_grass_blade_count {
                object
                    .raw_params
                    .insert("grass_blade_count".to_string(), blade_count.to_string());
            }

            if let Some(model_path) = infer_preview_model_path(&object, &model_index) {
                object.asset_hints.push(AssetRef {
                    path: model_path,
                    role: AssetRole::InferredPreviewModel,
                });
            }

            objects.push(object);
        }
    }

    if placement_files == 0 {
        issues.push(ValidationIssue::warning(
            "missing-placement-file",
            "No map/scene.bin placement file was found for this stage",
        ));
    }

    (objects, issues, lighting)
}

fn stage_model_index(assets: &[StageAsset]) -> Vec<(String, String)> {
    let mut models: Vec<_> = assets
        .iter()
        .filter(|asset| asset.kind == StageAssetKind::Model)
        .filter_map(|asset| {
            let path = asset.path.to_string_lossy().replace('\\', "/");
            let normalized = normalize_model_key(&path);
            (!normalized.is_empty()).then_some((path, normalized))
        })
        .collect();
    models.sort_by(|a, b| a.0.cmp(&b.0));
    models
}

#[derive(Clone)]
struct StageManagerResource {
    factory_name: String,
    chara_name: String,
}

fn build_actor_preview_catalog(
    base_root: &Path,
    assets: &[StageAsset],
    registry: &ObjectRegistry,
) -> (BTreeMap<String, ActorPreview>, Vec<ValidationIssue>) {
    let chara_folders = match load_obj_chara_folders(base_root) {
        Ok(folders) => folders,
        Err(_) if registry.enemy_managers.is_empty() => return (BTreeMap::new(), Vec::new()),
        Err(message) => {
            return (
                BTreeMap::new(),
                vec![ValidationIssue::warning(
                    "enemy-preview-catalog-unavailable",
                    message,
                )],
            );
        }
    };
    let model_index = stage_model_index(assets);
    let mut catalog = BTreeMap::new();
    let mut issues = Vec::new();
    let mut unresolved_factories = BTreeSet::new();

    for asset in assets
        .iter()
        .filter(|asset| asset.kind == StageAssetKind::Placement)
        .filter(|asset| {
            asset
                .path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase()
                .ends_with("/map/scene.bin")
        })
    {
        let Ok(bytes) = read_stage_asset_bytes(&asset.path) else {
            issues.push(ValidationIssue::warning(
                "enemy-preview-placement-read-failed",
                format!(
                    "Could not reread {} while resolving enemy previews",
                    asset.path.display()
                ),
            ));
            continue;
        };
        let Ok(records) = parse_jdrama_object_records(&bytes) else {
            issues.push(ValidationIssue::warning(
                "enemy-preview-placement-parse-failed",
                format!(
                    "Could not reparse {} while resolving enemy previews",
                    asset.path.display()
                ),
            ));
            continue;
        };
        let managers = records
            .iter()
            .filter_map(|record| {
                Some((
                    record.object_name.clone()?,
                    StageManagerResource {
                        factory_name: record.type_name.clone(),
                        chara_name: record.obj_manager_chara.clone()?,
                    },
                ))
            })
            .collect::<BTreeMap<_, _>>();

        for actor in &registry.enemy_actors {
            let factory_key = actor_preview_factory_key(&actor.factory_name);
            if let Some(model) = actor.indexed_models.iter().find(|model| model.index == 0) {
                if let Some(model_path) =
                    resolve_resource_model_path(&model.model_path, &model_index)
                {
                    catalog
                        .entry(factory_key.clone())
                        .or_insert_with(|| ActorPreview {
                            model_path,
                            load_flags: model.load_flags,
                            manager_factory: format!("{} actor", actor.factory_name),
                        });
                }
            }
            for manager_factory in &actor.manager_factories {
                let Some(manager_resource) = managers
                    .values()
                    .find(|resource| resource.factory_name.eq_ignore_ascii_case(manager_factory))
                else {
                    continue;
                };
                let Some(manager) = registry.find_enemy_manager(&manager_resource.factory_name)
                else {
                    continue;
                };
                let Some(folder) = chara_folders.get(&manager_resource.chara_name) else {
                    continue;
                };
                let Some(preview) =
                    resolve_manager_actor_preview(actor, manager, folder, &model_index)
                else {
                    continue;
                };
                catalog.entry(factory_key.clone()).or_insert(preview);
                break;
            }
            if catalog.contains_key(&factory_key) {
                continue;
            }
            for manager_factory in &actor.manager_factories {
                let Some(target_manager) = registry.find_enemy_manager(manager_factory) else {
                    continue;
                };
                for manager_resource in managers.values() {
                    let Some(stage_manager) =
                        registry.find_enemy_manager(&manager_resource.factory_name)
                    else {
                        continue;
                    };
                    if !manager_model_tables_are_aliases(target_manager, stage_manager) {
                        continue;
                    }
                    let Some(folder) = chara_folders.get(&manager_resource.chara_name) else {
                        continue;
                    };
                    let Some(preview) =
                        resolve_manager_actor_preview(actor, target_manager, folder, &model_index)
                    else {
                        continue;
                    };
                    catalog.insert(factory_key.clone(), preview);
                    break;
                }
                if catalog.contains_key(&factory_key) {
                    break;
                }
            }
        }

        for record in records.iter().filter(|record| record.transform.is_some()) {
            let has_manager_model_binding = record
                .live_actor_manager
                .as_ref()
                .and_then(|manager_name| managers.get(manager_name))
                .and_then(|resource| {
                    Some((
                        registry.find_enemy_manager(&resource.factory_name)?,
                        chara_folders.get(&resource.chara_name)?,
                    ))
                })
                .is_some();
            let has_actor_model_binding =
                registry
                    .find_enemy_actor(&record.type_name)
                    .is_some_and(|actor| {
                        (!actor.fallback_models.is_empty()
                            && record
                                .actor_character
                                .as_ref()
                                .and_then(|character| chara_folders.get(character))
                                .is_some())
                            || actor.named_models.iter().any(|model| {
                                record
                                    .object_name
                                    .as_ref()
                                    .is_some_and(|name| name == &model.actor_name)
                            })
                            || (!actor.indexed_models.is_empty()
                                && record.mario_modoki_telesa_imitation_index.is_some())
                    });
            let direct_actor_preview = (|| {
                let actor = registry.find_enemy_actor(&record.type_name)?;
                if let Some(selected) = actor
                    .named_models
                    .iter()
                    .find(|model| record.object_name.as_ref() == Some(&model.actor_name))
                {
                    return resolve_resource_model_path(&selected.model_path, &model_index).map(
                        |path| {
                            (
                                path,
                                selected.load_flags,
                                format!("{} actor", actor.factory_name),
                            )
                        },
                    );
                }
                let index = record.mario_modoki_telesa_imitation_index?;
                let exact = actor
                    .indexed_models
                    .iter()
                    .find(|model| model.index == index);
                let default = actor.indexed_models.iter().find(|model| model.index == 0);
                exact
                    .into_iter()
                    .chain(default.filter(|_| index != 0))
                    .find_map(|model| {
                        resolve_resource_model_path(&model.model_path, &model_index).map(|path| {
                            (
                                path,
                                model.load_flags,
                                format!("{} actor", actor.factory_name),
                            )
                        })
                    })
            })();
            let manager_preview = (|| {
                let manager_name = record.live_actor_manager.as_ref()?;
                let manager_resource = managers.get(manager_name)?;
                let manager = registry.find_enemy_manager(&manager_resource.factory_name)?;
                let folder = chara_folders.get(&manager_resource.chara_name)?;
                let actor = registry.find_enemy_actor(&record.type_name);
                resolve_manager_actor_preview(actor?, manager, folder, &model_index).map(
                    |preview| {
                        (
                            preview.model_path,
                            preview.load_flags,
                            preview.manager_factory,
                        )
                    },
                )
            })();
            let actor_preview = (|| {
                let actor = registry.find_enemy_actor(&record.type_name)?;
                let character = record.actor_character.as_ref()?;
                let folder = chara_folders.get(character)?;
                actor.fallback_models.iter().find_map(|model| {
                    resolve_chara_model_path(folder, &model.model_name, &model_index).map(|path| {
                        (
                            path,
                            model.load_flags,
                            format!("{} actor", actor.factory_name),
                        )
                    })
                })
            })();
            let Some((model_path, load_flags, manager_factory)) =
                direct_actor_preview.or(manager_preview).or(actor_preview)
            else {
                if has_manager_model_binding || has_actor_model_binding {
                    unresolved_factories.insert(record.type_name.clone());
                }
                continue;
            };
            let source = SourceLocation {
                path: asset.path.clone(),
                offset: Some(record.offset as u64),
                length: Some(record.size as u64),
            };
            let Some(key) = actor_preview_source_key(&source) else {
                continue;
            };
            let preview = ActorPreview {
                model_path,
                load_flags,
                manager_factory,
            };
            catalog.insert(key, preview.clone());
            let used_named_actor_preview = registry
                .find_enemy_actor(&record.type_name)
                .is_some_and(|actor| {
                    actor
                        .named_models
                        .iter()
                        .any(|model| record.object_name.as_ref() == Some(&model.actor_name))
                });
            let factory_preview = registry
                .find_enemy_actor(&record.type_name)
                .and_then(|actor| {
                    let model = actor.indexed_models.iter().find(|model| model.index == 0)?;
                    Some(ActorPreview {
                        model_path: resolve_resource_model_path(&model.model_path, &model_index)?,
                        load_flags: model.load_flags,
                        manager_factory: format!("{} actor", actor.factory_name),
                    })
                })
                .or_else(|| (!used_named_actor_preview).then(|| preview.clone()));
            if let Some(factory_preview) = factory_preview {
                catalog
                    .entry(actor_preview_factory_key(&record.type_name))
                    .or_insert(factory_preview);
            }
        }
    }
    if !unresolved_factories.is_empty() {
        issues.push(ValidationIssue::warning(
            "enemy-preview-model-unresolved",
            format!(
                "Could not resolve stage model assets for: {}",
                unresolved_factories
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    (catalog, issues)
}

fn load_obj_chara_folders(
    base_root: &Path,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let candidates = [
        base_root.join("files/data/scenecmn.bin"),
        base_root.join("data/scenecmn.bin"),
        base_root.join("scenecmn.bin"),
    ];
    let Some(path) = candidates.into_iter().find(|path| path.is_file()) else {
        return Err(format!(
            "Could not find data/scenecmn.bin under {} for enemy resource binding",
            base_root.display()
        ));
    };
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "Could not read {} for enemy resource binding: {error}",
            path.display()
        )
    })?;
    let records = parse_jdrama_object_records(&bytes).map_err(|error| {
        format!(
            "Could not parse {} for enemy resource binding: {error}",
            path.display()
        )
    })?;
    Ok(records
        .into_iter()
        .filter_map(|record| Some((record.object_name?, record.obj_chara_folder?)))
        .collect())
}

fn resolve_chara_model_path(
    folder: &str,
    model_name: &str,
    model_index: &[(String, String)],
) -> Option<String> {
    resolve_resource_model_path(&format!("{folder}/{model_name}"), model_index)
}

fn resolve_manager_actor_preview(
    actor: &EnemyActorDefinition,
    manager: &EnemyManagerDefinition,
    folder: &str,
    model_index: &[(String, String)],
) -> Option<ActorPreview> {
    actor_manager_model_candidates(actor, manager)
        .into_iter()
        .find_map(|model| {
            resolve_chara_model_path(folder, &model.model_name, model_index).map(|model_path| {
                ActorPreview {
                    model_path,
                    load_flags: model.load_flags,
                    manager_factory: manager.factory_name.clone(),
                }
            })
        })
}

fn actor_manager_model_candidates<'a>(
    actor: &'a EnemyActorDefinition,
    manager: &'a EnemyManagerDefinition,
) -> Vec<&'a EnemyModelDefinition> {
    if !actor.fallback_models.is_empty() {
        return actor.fallback_models.iter().collect();
    }
    if let Some(model_index) = actor.model_index.or(manager.model_index) {
        return manager.models.get(model_index).into_iter().collect();
    }
    if let Some(primary_model) = &actor.primary_model {
        return manager
            .models
            .iter()
            .filter(|model| model.model_name.eq_ignore_ascii_case(primary_model))
            .collect();
    }
    manager.models.first().into_iter().collect()
}

fn manager_model_tables_are_aliases(
    target: &EnemyManagerDefinition,
    stage: &EnemyManagerDefinition,
) -> bool {
    !target.models.is_empty()
        && target.models.len() == stage.models.len()
        && target
            .models
            .iter()
            .all(|model| !model.model_name.eq_ignore_ascii_case("default.bmd"))
        && target
            .models
            .iter()
            .zip(&stage.models)
            .all(|(target, stage)| {
                target.model_name.eq_ignore_ascii_case(&stage.model_name)
                    && target.load_flags == stage.load_flags
            })
}

fn resolve_resource_model_path(
    model_path: &str,
    model_index: &[(String, String)],
) -> Option<String> {
    let normalized = model_path.replace('\\', "/").to_ascii_lowercase();
    let suffix = normalized
        .strip_prefix("/scene/")
        .unwrap_or_else(|| normalized.trim_start_matches('/'));
    model_index
        .iter()
        .find(|(path, _)| {
            path.replace('\\', "/")
                .to_ascii_lowercase()
                .ends_with(&suffix)
        })
        .map(|(path, _)| path.clone())
}

fn actor_preview_source_key(source: &SourceLocation) -> Option<String> {
    Some(format!(
        "{}@{:x}",
        source.path.to_string_lossy().replace('\\', "/"),
        source.offset?
    ))
}

fn actor_preview_factory_key(factory_name: &str) -> String {
    format!("factory:{}", factory_name.to_ascii_lowercase())
}

fn infer_preview_model_path(
    object: &SceneObject,
    model_index: &[(String, String)],
) -> Option<String> {
    if let Some((directory, model_name)) = actor_preview_model_identity(&object.factory_name) {
        let resource_directory = format!("/{directory}/");
        if let Some((path, _)) = model_index.iter().find(|(path, _)| {
            let lower = path.to_ascii_lowercase();
            lower.contains(&resource_directory)
                && lower
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case(model_name))
        }) {
            return Some(path.clone());
        }
    }

    // TMapObjBase::load and the resource-selecting actors below read their
    // resource identity from the first placement stream string. Prefer that
    // authored basename over generic factory names.
    if object.factory_name.eq_ignore_ascii_case("MapObjBase")
        || object.factory_name.eq_ignore_ascii_case("Palm")
        || object.factory_name.eq_ignore_ascii_case("Shimmer")
        || object.factory_name.eq_ignore_ascii_case("ResetFruit")
    {
        if let Some(model_name) = object.raw_params.get("stream_string_0") {
            let key = normalize_model_key(model_name);
            if let Some(path) = exact_model_key_match(&key, model_index) {
                return Some(path);
            }
        }
    }

    let mut keys = Vec::new();
    keys.push(normalize_model_key(&object.factory_name));
    if let Some(class_name) = &object.class_name {
        keys.push(normalize_model_key(class_name));
        if let Some(short_name) = class_name.rsplit("::").next() {
            keys.push(normalize_model_key(short_name));
        }
    }
    if let Some(name) = object.raw_params.get("name") {
        keys.push(normalize_model_key(name));
    }
    keys.retain(|key| key.len() >= 3);
    keys.sort();
    keys.dedup();

    for key in &keys {
        if let Some(path) = exact_model_key_match(key, model_index) {
            return Some(path);
        }
    }

    for key in keys {
        if let Some(path) = fuzzy_model_key_match(&key, model_index) {
            return Some(path);
        }
    }

    None
}

fn actor_preview_model_identity(factory_name: &str) -> Option<(&'static str, &'static str)> {
    match factory_name.to_ascii_lowercase().as_str() {
        "npcmontem" => Some(("montem", "mom_model.bmd")),
        "npcmontema" => Some(("montema", "moma_model.bmd")),
        "npcmontemb" => Some(("montemb", "momb_model.bmd")),
        "npcmontemc" => Some(("montemc", "momc_model.bmd")),
        "npcmontemd" => Some(("montemd", "momd_model.bmd")),
        "npcmonteme" => Some(("monteme", "mome_model.bmd")),
        // These variants deliberately reuse another Monte model in the game.
        "npcmontemf" => Some(("montem", "mom_model.bmd")),
        "npcmontemg" => Some(("montemc", "momc_model.bmd")),
        "npcmontemh" => Some(("montema", "moma_model.bmd")),
        "npcmontew" => Some(("montew", "mow_model.bmd")),
        "npcmontewa" => Some(("montewa", "mowa_model.bmd")),
        "npcmontewb" => Some(("montewb", "mowb_model.bmd")),
        "npcmontewc" => Some(("montew", "mow_model.bmd")),
        "npcmarem" | "npcmarema" | "npcmaremb" | "npcmaremc" | "npcmaremd" => {
            Some(("marem", "marem.bmd"))
        }
        "npcmarew" | "npcmarewa" | "npcmarewb" => Some(("marew", "marew.bmd")),
        "npckinopio" => Some(("kinopio", "kinopio_body.bmd")),
        "npckinojii" => Some(("kinojii", "kinoji_body.bmd")),
        "npcpeach" => Some(("peach", "peach_model.bmd")),
        "npcraccoondog" => Some(("raccoondog", "tanuki.bmd")),
        "npcboard" => Some(("boardnpc", "boardnpc.bmd")),
        _ => None,
    }
}

fn exact_model_key_match(key: &str, model_index: &[(String, String)]) -> Option<String> {
    model_index
        .iter()
        .find(|(_, model_key)| model_key == key)
        .map(|(path, _)| path.clone())
}

fn fuzzy_model_key_match(key: &str, model_index: &[(String, String)]) -> Option<String> {
    let aliases = object_model_aliases(key);
    for alias in &aliases {
        if let Some((path, _)) = model_index.iter().find(|(path, model_key)| {
            let lower = path.to_ascii_lowercase();
            (lower.contains("!/mapobj/") || lower.contains("/scene/mapobj/")) && model_key == alias
        }) {
            return Some(path.clone());
        }
    }

    model_index
        .iter()
        .filter(|(path, _)| {
            let lower = path.to_ascii_lowercase();
            lower.contains("!/mapobj/") || lower.contains("/scene/mapobj/")
        })
        .find(|(_, model_key)| {
            model_key.contains(key)
                || key.contains(model_key.as_str())
                || aliases
                    .iter()
                    .any(|alias| model_key.contains(alias) || alias.contains(model_key.as_str()))
        })
        .map(|(path, _)| path.clone())
}

fn object_model_aliases(key: &str) -> Vec<&'static str> {
    let mut aliases = Vec::new();
    if key.contains("palm") {
        aliases.push("palmnormal");
    }
    if key.contains("manhole") {
        aliases.push("manhole");
    }
    if key.contains("kibako") || key.contains("crate") || key.contains("box") {
        aliases.push("kibako");
    }
    if key.contains("barrel") {
        aliases.push("barrelnormal");
    }
    if key.contains("coin") {
        aliases.push("coin");
    }
    aliases
}

fn normalize_model_key(value: &str) -> String {
    let value = value
        .rsplit("!/")
        .next()
        .unwrap_or(value)
        .rsplit('/')
        .next()
        .unwrap_or(value)
        .strip_suffix(".bmd")
        .or_else(|| value.strip_suffix(".bdl"))
        .unwrap_or(value);

    let mut key = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch.to_ascii_lowercase());
        }
    }

    for prefix in ["t", "m", "sm"] {
        if key.len() > prefix.len() + 3 && key.starts_with(prefix) {
            return key[prefix.len()..].to_string();
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_primary_object_light_and_ambient_by_runtime_names() {
        let lighting = StageLighting {
            lights: vec![JDramaLight {
                name: Some("太陽（オブジェクト）".to_string()),
                position: [1.0, 2.0, 3.0],
                color: [4, 5, 6, 255],
            }],
            ambients: vec![JDramaAmbient {
                name: Some("太陽アンビエント（オブジェクト）".to_string()),
                color: [7, 8, 9, 255],
            }],
        };
        assert_eq!(
            lighting.object_lighting(),
            Some(StageObjectLighting {
                position: [1.0, 2.0, 3.0],
                color: [4, 5, 6, 255],
                ambient: [7, 8, 9, 255],
            })
        );
    }

    #[test]
    fn detects_invalid_transform() {
        let mut doc = StageDocument {
            stage_id: "dolpic".to_string(),
            base_root: PathBuf::from("."),
            assets: vec![],
            objects: vec![],
            changed_files: BTreeMap::new(),
            registry: None,
            load_issues: Vec::new(),
            lighting: StageLighting::default(),
            actor_previews: BTreeMap::new(),
        };
        let mut object = SceneObject::new("obj-1", "coin");
        object.transform.translation[0] = f32::NAN;
        doc.add_object(object);

        let issues = doc.validate();
        assert!(issues.iter().any(|issue| issue.code == "invalid-transform"));
    }

    #[test]
    fn queues_editor_overlay_as_changed_file() {
        let mut doc = StageDocument {
            stage_id: "dolpic".to_string(),
            base_root: PathBuf::from("."),
            assets: vec![],
            objects: vec![SceneObject::new("obj-1", "coin")],
            changed_files: BTreeMap::new(),
            registry: None,
            load_issues: Vec::new(),
            lighting: StageLighting::default(),
            actor_previews: BTreeMap::new(),
        };

        doc.queue_editor_overlay_change().unwrap();
        assert!(doc
            .changed_files
            .contains_key(&PathBuf::from("editor/stages/dolpic.scene.json")));
    }

    #[test]
    fn rejects_project_paths_that_escape_the_output_root() {
        let mut doc = empty_document("dolpic");
        let err = doc
            .mark_changed_file(PathBuf::from("../outside.bin"), vec![1, 2, 3])
            .unwrap_err();
        assert!(matches!(err, SceneError::UnsafeProjectPath(_)));

        doc.stage_id = "../../outside".to_string();
        assert!(matches!(
            doc.queue_editor_overlay_change().unwrap_err(),
            SceneError::InvalidStageId(_)
        ));
    }

    #[test]
    fn project_export_replaces_stale_files() {
        let root = std::env::temp_dir().join(format!(
            "sms-editor-project-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut doc = empty_document("dolpic");
        doc.mark_changed_file("first.bin", vec![1]).unwrap();
        doc.save_project_folder(&root).unwrap();
        assert!(root.join("files/first.bin").exists());

        doc.changed_files.clear();
        doc.mark_changed_file("second.bin", vec![2]).unwrap();
        doc.save_project_folder(&root).unwrap();
        assert!(!root.join("files/first.bin").exists());
        assert!(root.join("files/second.bin").exists());
        assert!(root.join("sms-project.toml").exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_export_rejects_base_directory_overlap() {
        let root = std::env::temp_dir().join(format!(
            "sms-editor-overlap-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base_root = root.join("base");
        fs::create_dir_all(&base_root).unwrap();
        let mut document = empty_document("dolpic0");
        document.base_root = base_root.clone();

        assert!(matches!(
            document
                .save_project_folder(base_root.join("editor-project"))
                .unwrap_err(),
            SceneError::ProjectOverlapsBase(_)
        ));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn placement_parse_failures_are_reported_as_validation_errors() {
        let root = std::env::temp_dir().join(format!(
            "sms-editor-load-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let scene_dir = root.join("data/scene/dolpic0/map");
        fs::create_dir_all(&scene_dir).unwrap();
        fs::write(scene_dir.join("scene.bin"), b"not a JDrama stream").unwrap();

        let document = StageDocument::open(&root, "dolpic0").unwrap();
        assert!(document
            .validate()
            .iter()
            .any(|issue| issue.code == "placement-parse-failed"
                && issue.severity == ValidationSeverity::Error));

        fs::remove_dir_all(root).unwrap();
    }

    fn empty_document(stage_id: &str) -> StageDocument {
        StageDocument {
            stage_id: stage_id.to_string(),
            base_root: PathBuf::from("."),
            assets: vec![],
            objects: vec![],
            changed_files: BTreeMap::new(),
            registry: None,
            load_issues: Vec::new(),
            lighting: StageLighting::default(),
            actor_previews: BTreeMap::new(),
        }
    }

    #[test]
    fn resolves_npc_models_from_decomp_manager_resource_names() {
        let models = vec![
            (
                "stage.szs!/montema/moma_model.bmd".to_string(),
                "omamodel".to_string(),
            ),
            (
                "stage.szs!/kinopio/kinopio_body.bmd".to_string(),
                "kinopiobody".to_string(),
            ),
        ];
        let monte = SceneObject::new("monte", "NPCMonteMA");
        let kinopio = SceneObject::new("kinopio", "NPCKinopio");

        assert_eq!(
            infer_preview_model_path(&monte, &models).as_deref(),
            Some("stage.szs!/montema/moma_model.bmd")
        );
        assert_eq!(
            infer_preview_model_path(&kinopio, &models).as_deref(),
            Some("stage.szs!/kinopio/kinopio_body.bmd")
        );
    }

    #[test]
    #[ignore = "requires the extracted retail game and neighboring SMS decomp"]
    fn audits_all_retail_enemy_and_boss_previews() {
        let base_root = std::env::var_os("SMS_BASE_ROOT")
            .map(PathBuf::from)
            .expect("set SMS_BASE_ROOT to the extracted game's root");
        let decomp_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let registry = sms_schema::SchemaGenerator::new(decomp_root)
            .generate()
            .expect("generate enemy schema");
        let boss_factories = registry
            .enemy_actors
            .iter()
            .filter(|actor| {
                registry
                    .find_object(&actor.factory_name)
                    .is_some_and(|object| object.category == "Boss")
            })
            .map(|actor| actor.factory_name.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(boss_factories.len(), 21, "unexpected boss schema coverage");
        let archives = sms_formats::discover_scene_archives(&base_root)
            .expect("discover retail scene archives");
        assert_eq!(archives.len(), 107, "unexpected retail scene archive count");
        let mut placed = BTreeMap::<String, usize>::new();
        let mut rendered = BTreeMap::<String, usize>::new();
        let mut preview_models = BTreeMap::<(String, u32), String>::new();
        let mut factory_previews = BTreeSet::new();
        let mut boss_factory_previews = BTreeSet::new();
        let mut hino_model_paths = BTreeSet::new();
        let mut named_emario_count = 0;

        for archive in archives {
            let assets = sms_formats::mount_scene_archive(&archive.path)
                .unwrap_or_else(|error| panic!("mount {}: {error}", archive.path.display()));
            hino_model_paths.extend(
                assets
                    .iter()
                    .filter(|asset| asset.kind == StageAssetKind::Model)
                    .map(|asset| asset.path.to_string_lossy().replace('\\', "/"))
                    .filter(|path| path.to_ascii_lowercase().contains("hinokuri2")),
            );
            let (catalog, issues) = build_actor_preview_catalog(&base_root, &assets, &registry);
            assert!(
                issues.is_empty(),
                "enemy preview issues in {}: {issues:?}",
                archive.stage_id
            );
            for actor in &registry.enemy_actors {
                let factory = &actor.factory_name;
                if let Some(preview) = catalog.get(&actor_preview_factory_key(factory)) {
                    factory_previews.insert(factory.clone());
                    if boss_factories.contains(factory) {
                        boss_factory_previews.insert(factory.clone());
                    }
                    preview_models
                        .entry((preview.model_path.clone(), preview.load_flags))
                        .or_insert_with(|| factory.clone());
                }
            }
            for asset in assets.iter().filter(|asset| {
                asset.kind == StageAssetKind::Placement
                    && asset
                        .path
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_ascii_lowercase()
                        .ends_with("/map/scene.bin")
            }) {
                let bytes = read_stage_asset_bytes(&asset.path).expect("read placement");
                let records = parse_jdrama_object_records(&bytes).expect("parse placement");
                for record in
                    records.iter().filter(|record| {
                        record.transform.is_some()
                            && registry
                                .find_object(&record.type_name)
                                .is_some_and(|object| {
                                    matches!(object.category.as_str(), "Enemy" | "Boss")
                                        && !object.class_name.rsplit("::").next().is_some_and(
                                            |class_name| class_name.ends_with("Manager"),
                                        )
                                })
                    })
                {
                    assert!(
                        registry.find_enemy_actor(&record.type_name).is_some(),
                        "missing enemy actor schema for placed {}",
                        record.type_name
                    );
                    *placed.entry(record.type_name.clone()).or_default() += 1;
                    let source = SourceLocation {
                        path: asset.path.clone(),
                        offset: Some(record.offset as u64),
                        length: Some(record.size as u64),
                    };
                    let preview =
                        actor_preview_source_key(&source).and_then(|key| catalog.get(&key));
                    if let Some(preview) = preview {
                        *rendered.entry(record.type_name.clone()).or_default() += 1;
                        preview_models
                            .entry((preview.model_path.clone(), preview.load_flags))
                            .or_insert_with(|| record.type_name.clone());
                        if record.type_name == "EMario"
                            && record.object_name.as_deref() == Some("モンテマン")
                        {
                            assert!(preview
                                .model_path
                                .replace('\\', "/")
                                .to_ascii_lowercase()
                                .ends_with("/map/map/pad/monteman_model.bmd"));
                            assert_eq!(preview.load_flags, 0x1004_0000);
                            named_emario_count += 1;
                        } else if record.type_name == "EMario" {
                            assert!(preview
                                .model_path
                                .replace('\\', "/")
                                .to_ascii_lowercase()
                                .ends_with("/kagemario/default.bmd"));
                            assert_eq!(preview.load_flags, 0x1130_0000);
                        }
                        if record.type_name == "MarioModokiTelesa" {
                            let actor = registry.find_enemy_actor(&record.type_name).unwrap();
                            let index = record
                                .mario_modoki_telesa_imitation_index
                                .expect("typed imitation selector");
                            let expected = actor
                                .indexed_models
                                .iter()
                                .find(|model| model.index == index)
                                .or_else(|| {
                                    actor.indexed_models.iter().find(|model| model.index == 0)
                                })
                                .unwrap();
                            assert!(preview
                                .model_path
                                .replace('\\', "/")
                                .to_ascii_lowercase()
                                .ends_with(
                                    expected
                                        .model_path
                                        .trim_start_matches("/scene/")
                                        .to_ascii_lowercase()
                                        .as_str()
                                ));
                            assert_eq!(preview.load_flags, expected.load_flags);
                        }
                    }
                }
            }
        }

        assert_eq!(
            placed.len(),
            70,
            "unexpected placed enemy/boss factory count"
        );

        let unresolved = placed
            .iter()
            .filter_map(|(factory, count)| {
                let resolved = rendered.get(factory).copied().unwrap_or(0);
                (resolved != *count).then_some((factory.clone(), *count, resolved))
            })
            .collect::<Vec<_>>();
        eprintln!(
            "enemy preview factories: {} placed, {} fully resolved; unresolved={unresolved:?}",
            placed.len(),
            placed.len() - unresolved.len()
        );
        assert_eq!(
            unresolved
                .iter()
                .map(|(factory, _, _)| factory.as_str())
                .collect::<Vec<_>>(),
            ["EffectBiancoFunsui", "EffectPinnaFunsui"],
            "only the particle-only fountain actors may lack models"
        );
        assert_eq!(rendered.get("EMario"), placed.get("EMario"));
        assert_eq!(named_emario_count, 3);
        for factory in placed.keys().filter(|factory| {
            !matches!(factory.as_str(), "EffectBiancoFunsui" | "EffectPinnaFunsui")
        }) {
            assert!(
                factory_previews.contains(factory),
                "placed enemy {factory} lacks a source-less factory preview"
            );
        }
        for factory in ["EggGenerator", "WickedEggGenerator"] {
            assert!(
                factory_previews.contains(factory),
                "runtime enemy {factory} lacks a source-less factory preview"
            );
        }
        let missing_actor_factory_previews = registry
            .enemy_actors
            .iter()
            .map(|actor| actor.factory_name.clone())
            .filter(|factory| !factory_previews.contains(factory))
            .collect::<Vec<_>>();
        assert_eq!(
            missing_actor_factory_previews,
            [
                "EffectBiancoFunsui",
                "EffectEnemy",
                "EffectPinnaFunsui",
                "HinoKuri2",
                "KageMarioModoki",
                "NamekuriLauncher",
            ],
            "only particle-only or registered-but-unshipped enemy factories may lack a retail preview"
        );

        let unplaced_bosses = boss_factories
            .iter()
            .filter(|factory| !placed.contains_key(*factory))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            unplaced_bosses.len(),
            6,
            "unexpected runtime-created boss coverage: {unplaced_bosses:?}"
        );
        assert!(
            hino_model_paths.is_empty(),
            "HinoKuri2 unexpectedly has retail model resources: {hino_model_paths:?}"
        );
        let missing_boss_previews = boss_factories
            .difference(&boss_factory_previews)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            missing_boss_previews,
            ["HinoKuri2"],
            "only the unshipped HinoKuri2 factory may lack a retail preview"
        );

        for (factory, count) in &placed {
            let is_boss = registry
                .find_object(factory)
                .is_some_and(|object| object.category == "Boss");
            if is_boss {
                assert_eq!(
                    rendered.get(factory),
                    Some(count),
                    "unresolved boss {factory}"
                );
            }
        }

        for ((model_path, load_flags), factory) in preview_models {
            let bytes = read_stage_asset_bytes(&model_path).unwrap_or_else(|error| {
                panic!("read {factory} preview {model_path} ({load_flags:#010x}): {error}")
            });
            let file = sms_formats::J3dFile::parse(&bytes)
                .unwrap_or_else(|error| panic!("parse {factory} preview {model_path}: {error}"));
            let geometry = file
                .geometry_preview_with_loader_flags(load_flags)
                .unwrap_or_else(|error| {
                    panic!("prepare {factory} preview {model_path} ({load_flags:#010x}): {error}")
                });
            assert!(
                !geometry.triangles.is_empty(),
                "empty {factory} preview {model_path} ({load_flags:#010x})"
            );
        }
    }

    #[test]
    fn resolves_enemy_model_from_exact_chara_folder_and_decomp_model_name() {
        let models = vec![
            (
                "stage.szs!/gatekeeper/gene_pakkun_model1.bmd".to_string(),
                "genepakkunmodel1".to_string(),
            ),
            (
                "stage.szs!/gatekeeper/stamp_keeper_model1.bmd".to_string(),
                "stampkeepermodel1".to_string(),
            ),
        ];
        assert_eq!(
            resolve_chara_model_path("/scene/gatekeeper", "gene_pakkun_model1.bmd", &models)
                .as_deref(),
            Some("stage.szs!/gatekeeper/gene_pakkun_model1.bmd")
        );
    }

    #[test]
    fn actor_default_model_flags_override_the_manager_table() {
        let actor = EnemyActorDefinition {
            factory_name: "EMario".to_string(),
            class_name: "TEMario".to_string(),
            model_index: None,
            fallback_models: vec![sms_schema::EnemyModelDefinition {
                model_name: "default.bmd".to_string(),
                load_flags: 0x1130_0000,
                source_file: "src/Enemy/emario.cpp".to_string(),
            }],
            primary_model: None,
            named_models: Vec::new(),
            indexed_models: Vec::new(),
            manager_factories: vec!["EMarioManager".to_string()],
        };
        let manager = EnemyManagerDefinition {
            factory_name: "EMarioManager".to_string(),
            class_name: "TEMarioManager".to_string(),
            model_index: None,
            spawned_actor_class: Some("TEMario".to_string()),
            models: vec![sms_schema::EnemyModelDefinition {
                model_name: "default.bmd".to_string(),
                load_flags: 0x1021_0000,
                source_file: "src/Strategic/ObjModel.cpp".to_string(),
            }],
        };
        let models = vec![(
            "stage.szs!/kagemario/default.bmd".to_string(),
            "kagemariodefault".to_string(),
        )];

        let preview =
            resolve_manager_actor_preview(&actor, &manager, "/scene/kagemario", &models).unwrap();

        assert_eq!(preview.model_path, "stage.szs!/kagemario/default.bmd");
        assert_eq!(preview.load_flags, 0x1130_0000);
        assert_eq!(preview.manager_factory, "EMarioManager");
    }

    #[test]
    fn indexed_enemy_variant_uses_its_decomp_model_slot() {
        let actor = EnemyActorDefinition {
            factory_name: "ButterflyC".to_string(),
            class_name: "TButterfloid".to_string(),
            model_index: Some(2),
            fallback_models: Vec::new(),
            primary_model: None,
            named_models: Vec::new(),
            indexed_models: Vec::new(),
            manager_factories: vec!["ButterflyManager".to_string()],
        };
        let manager = EnemyManagerDefinition {
            factory_name: "ButterflyManager".to_string(),
            class_name: "TButterfloidManager".to_string(),
            model_index: None,
            spawned_actor_class: None,
            models: ["butterflyA.bmd", "butterflyB.bmd", "butterflyC.bmd"]
                .into_iter()
                .map(|model_name| sms_schema::EnemyModelDefinition {
                    model_name: model_name.to_string(),
                    load_flags: 0x1021_0000,
                    source_file: "src/Animal/Butterfly.cpp".to_string(),
                })
                .collect(),
        };
        let models = ["butterflyA.bmd", "butterflyB.bmd", "butterflyC.bmd"]
            .into_iter()
            .map(|model_name| {
                (
                    format!("stage.szs!/butterfly/{model_name}"),
                    normalize_model_key(model_name),
                )
            })
            .collect::<Vec<_>>();

        let preview =
            resolve_manager_actor_preview(&actor, &manager, "/scene/butterfly", &models).unwrap();

        assert_eq!(preview.model_path, "stage.szs!/butterfly/butterflyC.bmd");
        assert_eq!(preview.load_flags, 0x1021_0000);
    }

    #[test]
    fn managerless_runtime_boss_reuses_an_exact_manager_model_table() {
        let actor = EnemyActorDefinition {
            factory_name: "LimitKoopa".to_string(),
            class_name: "TLimitKoopa".to_string(),
            model_index: None,
            fallback_models: Vec::new(),
            primary_model: None,
            named_models: Vec::new(),
            indexed_models: Vec::new(),
            manager_factories: vec!["LimitKoopaManager".to_string()],
        };
        let target_manager = EnemyManagerDefinition {
            factory_name: "LimitKoopaManager".to_string(),
            class_name: "TLimitKoopaManager".to_string(),
            model_index: None,
            spawned_actor_class: None,
            models: vec![sms_schema::EnemyModelDefinition {
                model_name: "koopa_model.bmd".to_string(),
                load_flags: 0x1424_0000,
                source_file: "src/Enemy/limitkoopa.cpp".to_string(),
            }],
        };
        let stage_manager = EnemyManagerDefinition {
            factory_name: "KoopaManager".to_string(),
            class_name: "TKoopaManager".to_string(),
            model_index: None,
            spawned_actor_class: None,
            models: vec![sms_schema::EnemyModelDefinition {
                model_name: "koopa_model.bmd".to_string(),
                load_flags: 0x1424_0000,
                source_file: "src/Enemy/koopa.cpp".to_string(),
            }],
        };
        let models = vec![(
            "coronaBoss.szs!/koopa/koopa_model.bmd".to_string(),
            "koopakoopamodel".to_string(),
        )];

        assert!(manager_model_tables_are_aliases(
            &target_manager,
            &stage_manager
        ));
        let preview =
            resolve_manager_actor_preview(&actor, &target_manager, "/scene/koopa", &models)
                .unwrap();
        assert_eq!(preview.model_path, "coronaBoss.szs!/koopa/koopa_model.bmd");
        assert_eq!(preview.load_flags, 0x1424_0000);

        let mut generic_target = target_manager.clone();
        generic_target.models[0].model_name = "default.bmd".to_string();
        let mut generic_stage = stage_manager;
        generic_stage.models[0].model_name = "default.bmd".to_string();
        assert!(!manager_model_tables_are_aliases(
            &generic_target,
            &generic_stage
        ));
    }

    #[test]
    fn actor_primary_model_is_not_substituted_with_a_base_manager_model() {
        let actor = EnemyActorDefinition {
            factory_name: "HaneHamuKuri2".to_string(),
            class_name: "THaneHamuKuri2".to_string(),
            model_index: None,
            fallback_models: Vec::new(),
            primary_model: Some("hanekuri.bmd".to_string()),
            named_models: Vec::new(),
            indexed_models: Vec::new(),
            manager_factories: vec!["HaneHamuKuriManager".to_string()],
        };
        let base_manager = EnemyManagerDefinition {
            factory_name: "HamuKuriManager".to_string(),
            class_name: "THamuKuriManager".to_string(),
            model_index: None,
            spawned_actor_class: Some("THamuKuri".to_string()),
            models: vec![sms_schema::EnemyModelDefinition {
                model_name: "default.bmd".to_string(),
                load_flags: 0x1022_0000,
                source_file: "src/Enemy/hamukuri.cpp".to_string(),
            }],
        };
        let models = vec![(
            "stage.szs!/hamukuri/default.bmd".to_string(),
            "hamukuridefault".to_string(),
        )];

        assert!(
            resolve_manager_actor_preview(&actor, &base_manager, "/scene/hamukuri", &models)
                .is_none()
        );
    }

    #[test]
    fn source_less_spawned_enemy_reuses_stage_factory_preview() {
        let mut document = empty_document("bianco0");
        document.actor_previews.insert(
            actor_preview_factory_key("HamuKuri"),
            ActorPreview {
                model_path: "bianco0.szs!/hamukuri/default.bmd".to_string(),
                load_flags: 0x1022_0000,
                manager_factory: "HamuKuriManager".to_string(),
            },
        );
        let spawned = SceneObject::new("spawned", "HamuKuri");

        let preview = document.actor_preview(&spawned).unwrap();
        assert_eq!(preview.model_path, "bianco0.szs!/hamukuri/default.bmd");
        assert_eq!(preview.load_flags, 0x1022_0000);
    }

    #[test]
    fn replacing_registry_rebuilds_catalog_and_replaces_catalog_issues() {
        let mut document = empty_document("bianco0");
        document.actor_previews.insert(
            actor_preview_factory_key("HamuKuri"),
            ActorPreview {
                model_path: "stale.bmd".to_string(),
                load_flags: 0,
                manager_factory: "stale".to_string(),
            },
        );
        document.load_issues.push(ValidationIssue::warning(
            "enemy-preview-stale",
            "old catalog issue",
        ));
        document
            .load_issues
            .push(ValidationIssue::warning("unrelated", "keep me"));

        document.set_registry(ObjectRegistry::default());

        assert!(document.actor_previews.is_empty());
        assert_eq!(document.load_issues.len(), 1);
        assert_eq!(document.load_issues[0].code, "unrelated");
    }

    #[test]
    fn special_monte_variants_reuse_the_game_model_directory() {
        let models = vec![(
            "stage.szs!/montema/moma_model.bmd".to_string(),
            "omamodel".to_string(),
        )];
        let object = SceneObject::new("map-shop", "NPCMonteMH");

        assert_eq!(
            infer_preview_model_path(&object, &models).as_deref(),
            Some("stage.szs!/montema/moma_model.bmd")
        );
    }

    #[test]
    fn map_obj_base_uses_the_resource_basename_stored_in_its_placement_stream() {
        let models = vec![(
            "stage.szs!/mapobj/stagefixture.bmd".to_string(),
            "stagefixture".to_string(),
        )];
        let mut object = SceneObject::new("generic map object", "MapObjBase");
        object
            .raw_params
            .insert("stream_string_0".to_string(), "StageFixture".to_string());

        assert_eq!(
            infer_preview_model_path(&object, &models).as_deref(),
            Some("stage.szs!/mapobj/stagefixture.bmd")
        );
    }

    #[test]
    fn shimmer_uses_the_model_basename_stored_in_its_placement_stream() {
        let models = vec![
            (
                "stage.szs!/mapobj/shimmerhi.bmd".to_string(),
                "shimmerhi".to_string(),
            ),
            (
                "stage.szs!/mapobj/shimmerlow.bmd".to_string(),
                "shimmerlow".to_string(),
            ),
            (
                "stage.szs!/mapobj/shimmerlowfar.bmd".to_string(),
                "shimmerlowfar".to_string(),
            ),
        ];
        let mut object = SceneObject::new("heatwave", "Shimmer");
        object
            .raw_params
            .insert("stream_string_0".to_string(), "ShimmerLowFar".to_string());

        assert_eq!(
            infer_preview_model_path(&object, &models).as_deref(),
            Some("stage.szs!/mapobj/shimmerlowfar.bmd")
        );

        object
            .raw_params
            .insert("stream_string_0".to_string(), "ShimmerHi".to_string());
        assert_eq!(
            infer_preview_model_path(&object, &models).as_deref(),
            Some("stage.szs!/mapobj/shimmerhi.bmd")
        );
    }

    #[test]
    fn palm_uses_the_model_basename_stored_in_its_placement_stream() {
        let models = vec![
            (
                "stage.szs!/mapobj/palmnormal.bmd".to_string(),
                "palmnormal".to_string(),
            ),
            (
                "stage.szs!/mapobj/palmleaf.bmd".to_string(),
                "palmleaf".to_string(),
            ),
        ];
        let mut object = SceneObject::new("PalmLeaf 2", "Palm");
        object
            .raw_params
            .insert("name".to_string(), "PalmLeaf 2".to_string());
        object
            .raw_params
            .insert("stream_string_0".to_string(), "palmLeaf".to_string());

        assert_eq!(
            infer_preview_model_path(&object, &models).as_deref(),
            Some("stage.szs!/mapobj/palmleaf.bmd")
        );
    }

    #[test]
    fn reset_fruits_use_the_model_basename_stored_in_their_placement_stream() {
        let models = vec![
            (
                "stage.szs!/mapobj/fruitbanana.bmd".to_string(),
                "fruitbanana".to_string(),
            ),
            (
                "stage.szs!/mapobj/fruitcoconut.bmd".to_string(),
                "fruitcoconut".to_string(),
            ),
            (
                "stage.szs!/mapobj/fruitdurian.bmd".to_string(),
                "fruitdurian".to_string(),
            ),
            (
                "stage.szs!/mapobj/fruitpapaya.bmd".to_string(),
                "fruitpapaya".to_string(),
            ),
            (
                "stage.szs!/mapobj/fruitpine.bmd".to_string(),
                "fruitpine".to_string(),
            ),
        ];

        for fruit in [
            "FruitBanana",
            "FruitCoconut",
            "FruitDurian",
            "FruitPapaya",
            "FruitPine",
        ] {
            let mut object = SceneObject::new(fruit, "ResetFruit");
            object
                .raw_params
                .insert("stream_string_0".to_string(), fruit.to_string());

            assert_eq!(
                infer_preview_model_path(&object, &models).as_deref(),
                Some(format!("stage.szs!/mapobj/{}.bmd", fruit.to_ascii_lowercase()).as_str())
            );
        }
    }
}
